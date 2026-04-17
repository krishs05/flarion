use std::ops::ControlFlow;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::sync::mpsc;
use tracing::{error, info};

use crate::api::types::{
    ChatCompletionChoice, ChatCompletionChunk, ChatCompletionChunkChoice, ChatCompletionRequest,
    ChatCompletionResponse, ChatDelta, ChatMessage, Usage,
};
use crate::config::ModelConfig;
use crate::error::EngineError;

use super::inference::{GenerationEvent, ModelAdapter, completion_id, unix_timestamp};
use super::protocol::{DispatchOutcome, WorkerCommand};

/// OS-thread entry point. Generic over the adapter so tests can inject
/// `ScriptedAdapter` instead of the real `LlamaAdapter`.
pub(super) fn run<M: ModelAdapter>(
    config: ModelConfig,
    mut cmd_rx: mpsc::Receiver<WorkerCommand>,
    poisoned: Arc<AtomicBool>,
    mut adapter: M,
) {
    // First command must be Load. Anything else is a caller-side API misuse.
    let (load_main_gpu, load_devices, load_split_mode, load_ack) = match cmd_rx.blocking_recv() {
        Some(WorkerCommand::Load {
            main_gpu,
            devices,
            split_mode,
            ack,
        }) => (main_gpu, devices, split_mode, ack),
        Some(_) | None => {
            error!("worker: first command was not Load or channel closed");
            return;
        }
    };

    match adapter.load(&config, load_main_gpu, &load_devices, load_split_mode) {
        Ok(()) => {
            let _ = load_ack.send(Ok(()));
            crate::metrics::set_backend_poisoned(&config.id, false);
            info!(model_id = %config.id, "llama worker started");
        }
        Err(e) => {
            let _ = load_ack.send(Err(e));
            return;
        }
    }

    while let Some(cmd) = cmd_rx.blocking_recv() {
        let outcome =
            std::panic::catch_unwind(AssertUnwindSafe(|| dispatch(&mut adapter, &config, cmd)));
        match outcome {
            Ok(DispatchOutcome::Continue) => {}
            Ok(DispatchOutcome::Shutdown) => break,
            Err(_) => {
                poisoned.store(true, Ordering::Release);
                error!(model_id = %config.id, "llama worker panicked, marking backend poisoned");
                crate::metrics::set_backend_poisoned(&config.id, true);
                break;
            }
        }
    }

    drop(adapter);
    info!(model_id = %config.id, "llama worker exited cleanly");
}

/// Handle one command; returns `Shutdown` only for `WorkerCommand::Shutdown`.
fn dispatch<M: ModelAdapter>(
    adapter: &mut M,
    config: &ModelConfig,
    cmd: WorkerCommand,
) -> DispatchOutcome {
    match cmd {
        WorkerCommand::Load { .. } => {
            error!("worker: received duplicate Load command");
            DispatchOutcome::Continue
        }
        WorkerCommand::Chat { request, ack } => {
            let result = handle_chat(adapter, config, request);
            let _ = ack.send(result);
            DispatchOutcome::Continue
        }
        WorkerCommand::ChatStream {
            request,
            chunks,
            done,
        } => {
            let result = handle_chat_stream(adapter, config, request, &chunks);
            let _ = done.send(result);
            DispatchOutcome::Continue
        }
        WorkerCommand::Shutdown { ack } => {
            let _ = ack.send(());
            DispatchOutcome::Shutdown
        }
    }
}

fn handle_chat<M: ModelAdapter>(
    adapter: &mut M,
    config: &ModelConfig,
    request: ChatCompletionRequest,
) -> Result<ChatCompletionResponse, EngineError> {
    let id = completion_id();
    let created = unix_timestamp();
    let model_id = request.model.clone();

    // Non-streaming: adapter accumulates text internally, callback is a no-op.
    let result = adapter.generate(&request, config, &mut |_event: GenerationEvent<'_>| {
        ControlFlow::Continue(())
    })?;

    Ok(ChatCompletionResponse {
        id,
        object: "chat.completion".into(),
        created,
        model: model_id,
        choices: vec![ChatCompletionChoice {
            index: 0,
            message: ChatMessage {
                role: "assistant".into(),
                content: result.text,
            },
            finish_reason: result.finish_reason.as_str().to_string(),
        }],
        usage: Usage {
            prompt_tokens: result.prompt_tokens,
            completion_tokens: result.completion_tokens,
            total_tokens: result.prompt_tokens + result.completion_tokens,
        },
    })
}

fn handle_chat_stream<M: ModelAdapter>(
    adapter: &mut M,
    config: &ModelConfig,
    request: ChatCompletionRequest,
    chunks: &tokio::sync::mpsc::Sender<ChatCompletionChunk>,
) -> Result<(), EngineError> {
    let id = completion_id();
    let created = unix_timestamp();
    let model_id = request.model.clone();

    // Opening chunk announces the assistant role, matching OpenAI's contract.
    let role_chunk = ChatCompletionChunk {
        id: id.clone(),
        object: "chat.completion.chunk".into(),
        created,
        model: model_id.clone(),
        choices: vec![ChatCompletionChunkChoice {
            index: 0,
            delta: ChatDelta {
                role: Some("assistant".into()),
                content: None,
            },
            finish_reason: None,
        }],
    };
    // Client already gone → treat as canceled, not an error.
    if chunks.blocking_send(role_chunk).is_err() {
        return Ok(());
    }

    let id_for_cb = id.clone();
    let model_for_cb = model_id.clone();

    let result = adapter.generate(&request, config, &mut |event: GenerationEvent<'_>| {
        let GenerationEvent::Token { text } = event;
        let chunk = ChatCompletionChunk {
            id: id_for_cb.clone(),
            object: "chat.completion.chunk".into(),
            created,
            model: model_for_cb.clone(),
            choices: vec![ChatCompletionChunkChoice {
                index: 0,
                delta: ChatDelta {
                    role: None,
                    content: Some(text.to_string()),
                },
                finish_reason: None,
            }],
        };

        // Try a non-blocking send first; only fall back to blocking_send when
        // the channel is full (backpressure). A closed channel means the
        // client disconnected, so cancel generation.
        use tokio::sync::mpsc::error::TrySendError;
        match chunks.try_send(chunk) {
            Ok(()) => {}
            Err(TrySendError::Full(c)) => {
                if chunks.blocking_send(c).is_err() {
                    return ControlFlow::Break(());
                }
            }
            Err(TrySendError::Closed(_)) => {
                return ControlFlow::Break(());
            }
        }

        if chunks.is_closed() {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    })?;

    // Emit a final chunk carrying finish_reason only when we actually
    // finished generating (canceled streams don't get a trailer).
    if !result.canceled {
        let final_chunk = ChatCompletionChunk {
            id,
            object: "chat.completion.chunk".into(),
            created,
            model: model_id,
            choices: vec![ChatCompletionChunkChoice {
                index: 0,
                delta: ChatDelta {
                    role: None,
                    content: None,
                },
                finish_reason: Some(result.finish_reason.as_str().into()),
            }],
        };
        let _ = chunks.blocking_send(final_chunk);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::BackendType;
    use crate::engine::llama::inference::test_adapter::{ScriptStep, ScriptedAdapter};
    use crate::engine::llama::protocol::WorkerCommand;
    use std::path::PathBuf;
    use tokio::sync::oneshot;

    fn load_cmd(ack: oneshot::Sender<Result<(), EngineError>>) -> WorkerCommand {
        WorkerCommand::Load {
            main_gpu: 0,
            devices: vec![],
            split_mode: llama_cpp_2::model::params::LlamaSplitMode::None,
            ack,
        }
    }

    fn test_config() -> ModelConfig {
        ModelConfig {
            id: "test-model".into(),
            backend: BackendType::Local,
            path: Some(PathBuf::from("/tmp/test.gguf")),
            context_size: 4096,
            gpu_layers: 0,
            threads: None,
            batch_size: None,
            seed: None,
            api_key: None,
            base_url: None,
            upstream_model: None,
            timeout_secs: None,
            max_tokens_cap: None,
            lazy: false,
            vram_mb: None,
            pin: false,
            gpus: vec![],
        }
    }

    fn spawn_worker(
        adapter: ScriptedAdapter,
    ) -> (
        mpsc::Sender<WorkerCommand>,
        Arc<AtomicBool>,
        std::thread::JoinHandle<()>,
    ) {
        let (cmd_tx, cmd_rx) = mpsc::channel(32);
        let poisoned = Arc::new(AtomicBool::new(false));
        let poisoned_clone = poisoned.clone();
        let config = test_config();
        let handle = std::thread::spawn(move || {
            run(config, cmd_rx, poisoned_clone, adapter);
        });
        (cmd_tx, poisoned, handle)
    }

    #[tokio::test]
    async fn worker_load_success() {
        let adapter = ScriptedAdapter::new(vec![]);
        let (cmd_tx, poisoned, handle) = spawn_worker(adapter);

        let (ack_tx, ack_rx) = oneshot::channel();
        cmd_tx
            .send(load_cmd(ack_tx))
            .await
            .unwrap();
        let result = ack_rx.await.unwrap();
        assert!(result.is_ok());
        assert!(!poisoned.load(Ordering::SeqCst));

        let (sd_tx, sd_rx) = oneshot::channel();
        cmd_tx
            .send(WorkerCommand::Shutdown { ack: sd_tx })
            .await
            .unwrap();
        sd_rx.await.unwrap();
        handle.join().unwrap();
    }

    #[tokio::test]
    async fn worker_load_failure_exits_thread() {
        let adapter = ScriptedAdapter::new(vec![ScriptStep::LoadFails("boom".into())]);
        let (cmd_tx, poisoned, handle) = spawn_worker(adapter);

        let (ack_tx, ack_rx) = oneshot::channel();
        cmd_tx
            .send(load_cmd(ack_tx))
            .await
            .unwrap();
        let result = ack_rx.await.unwrap();
        assert!(matches!(
            result,
            Err(crate::error::EngineError::ModelLoadFailed(_))
        ));

        // Thread exits itself on load failure; poisoned only flips on panic.
        handle.join().unwrap();
        assert!(!poisoned.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn worker_chat_non_streaming_returns_response() {
        let adapter = ScriptedAdapter::new(vec![
            ScriptStep::Token("Hello".into()),
            ScriptStep::Token(" world".into()),
            ScriptStep::Eog,
        ]);
        let (cmd_tx, _poisoned, handle) = spawn_worker(adapter);

        let (load_tx, load_rx) = oneshot::channel();
        cmd_tx
            .send(load_cmd(load_tx))
            .await
            .unwrap();
        load_rx.await.unwrap().unwrap();

        let (chat_tx, chat_rx) = oneshot::channel();
        let request = crate::api::types::ChatCompletionRequest {
            model: "test-model".into(),
            messages: vec![crate::api::types::ChatMessage {
                role: "user".into(),
                content: "hi".into(),
            }],
            stream: false,
            temperature: 0.7,
            top_p: 0.9,
            max_tokens: 16,
            stop: Vec::new(),
            seed: None,
        };
        cmd_tx
            .send(WorkerCommand::Chat {
                request,
                ack: chat_tx,
            })
            .await
            .unwrap();

        let resp = chat_rx.await.unwrap().unwrap();
        assert_eq!(resp.choices[0].message.content, "Hello world");
        assert_eq!(resp.choices[0].finish_reason, "stop");
        assert_eq!(resp.usage.completion_tokens, 2);

        let (sd_tx, sd_rx) = oneshot::channel();
        cmd_tx
            .send(WorkerCommand::Shutdown { ack: sd_tx })
            .await
            .unwrap();
        sd_rx.await.unwrap();
        handle.join().unwrap();
    }

    #[tokio::test]
    async fn worker_chat_stream_emits_all_chunks() {
        let adapter = ScriptedAdapter::new(vec![
            ScriptStep::Token("a".into()),
            ScriptStep::Token("b".into()),
            ScriptStep::Eog,
        ]);
        let (cmd_tx, _poisoned, handle) = spawn_worker(adapter);

        let (load_tx, load_rx) = oneshot::channel();
        cmd_tx
            .send(load_cmd(load_tx))
            .await
            .unwrap();
        load_rx.await.unwrap().unwrap();

        let (chunk_tx, mut chunk_rx) = tokio::sync::mpsc::channel(16);
        let (done_tx, done_rx) = oneshot::channel();
        let request = crate::api::types::ChatCompletionRequest {
            model: "test-model".into(),
            messages: vec![crate::api::types::ChatMessage {
                role: "user".into(),
                content: "go".into(),
            }],
            stream: true,
            temperature: 0.7,
            top_p: 0.9,
            max_tokens: 16,
            stop: Vec::new(),
            seed: None,
        };
        cmd_tx
            .send(WorkerCommand::ChatStream {
                request,
                chunks: chunk_tx,
                done: done_tx,
            })
            .await
            .unwrap();

        let mut content = String::new();
        let mut saw_finish = false;
        while let Some(chunk) = chunk_rx.recv().await {
            if let Some(c) = &chunk.choices[0].delta.content {
                content.push_str(c);
            }
            if chunk.choices[0].finish_reason.is_some() {
                saw_finish = true;
            }
        }
        assert_eq!(content, "ab");
        assert!(saw_finish);
        done_rx.await.unwrap().unwrap();

        let (sd_tx, sd_rx) = oneshot::channel();
        cmd_tx
            .send(WorkerCommand::Shutdown { ack: sd_tx })
            .await
            .unwrap();
        sd_rx.await.unwrap();
        handle.join().unwrap();
    }

    #[tokio::test]
    async fn worker_chat_stream_client_disconnect_cancels() {
        let adapter = ScriptedAdapter::new(vec![
            ScriptStep::Token("one".into()),
            ScriptStep::Token("two".into()),
            ScriptStep::Token("three".into()),
            ScriptStep::Token("four".into()),
            ScriptStep::Eog,
        ]);
        // Must clone before `adapter` is moved into spawn_worker.
        let sampled_counter = adapter.sampled.clone();
        let (cmd_tx, _poisoned, handle) = spawn_worker(adapter);

        let (load_tx, load_rx) = oneshot::channel();
        cmd_tx
            .send(load_cmd(load_tx))
            .await
            .unwrap();
        load_rx.await.unwrap().unwrap();

        // Tiny channel so subsequent sends hit Closed quickly after drop.
        let (chunk_tx, mut chunk_rx) = tokio::sync::mpsc::channel(2);
        let (done_tx, done_rx) = oneshot::channel();
        let request = crate::api::types::ChatCompletionRequest {
            model: "test-model".into(),
            messages: vec![crate::api::types::ChatMessage {
                role: "user".into(),
                content: "go".into(),
            }],
            stream: true,
            temperature: 0.7,
            top_p: 0.9,
            max_tokens: 16,
            stop: Vec::new(),
            seed: None,
        };
        cmd_tx
            .send(WorkerCommand::ChatStream {
                request,
                chunks: chunk_tx,
                done: done_tx,
            })
            .await
            .unwrap();

        // Drop the receiver to simulate the client disconnecting mid-stream.
        let _first = chunk_rx.recv().await.unwrap();
        drop(chunk_rx);

        let result = done_rx.await.unwrap();
        assert!(result.is_ok(), "done should resolve Ok even on cancel");

        assert!(
            sampled_counter.load(Ordering::SeqCst) < 4,
            "expected early cancellation, got {} sampled",
            sampled_counter.load(Ordering::SeqCst)
        );

        let (sd_tx, sd_rx) = oneshot::channel();
        cmd_tx
            .send(WorkerCommand::Shutdown { ack: sd_tx })
            .await
            .unwrap();
        sd_rx.await.unwrap();
        handle.join().unwrap();
    }

    #[tokio::test]
    async fn worker_shutdown_drains_queue_and_fifo() {
        // Three Chat commands then Shutdown: all three must complete in FIFO
        // order before the Shutdown ack fires.
        let adapter = ScriptedAdapter::new(vec![
            ScriptStep::Token("1".into()),
            ScriptStep::Eog,
            ScriptStep::Token("2".into()),
            ScriptStep::Eog,
            ScriptStep::Token("3".into()),
            ScriptStep::Eog,
        ]);
        let (cmd_tx, _poisoned, handle) = spawn_worker(adapter);

        let (load_tx, load_rx) = oneshot::channel();
        cmd_tx
            .send(load_cmd(load_tx))
            .await
            .unwrap();
        load_rx.await.unwrap().unwrap();

        let mk_req = |tag: &str| crate::api::types::ChatCompletionRequest {
            model: "test-model".into(),
            messages: vec![crate::api::types::ChatMessage {
                role: "user".into(),
                content: tag.into(),
            }],
            stream: false,
            temperature: 0.7,
            top_p: 0.9,
            max_tokens: 16,
            stop: Vec::new(),
            seed: None,
        };

        let (tx1, rx1) = oneshot::channel();
        let (tx2, rx2) = oneshot::channel();
        let (tx3, rx3) = oneshot::channel();
        cmd_tx
            .send(WorkerCommand::Chat {
                request: mk_req("a"),
                ack: tx1,
            })
            .await
            .unwrap();
        cmd_tx
            .send(WorkerCommand::Chat {
                request: mk_req("b"),
                ack: tx2,
            })
            .await
            .unwrap();
        cmd_tx
            .send(WorkerCommand::Chat {
                request: mk_req("c"),
                ack: tx3,
            })
            .await
            .unwrap();

        let (sd_tx, sd_rx) = oneshot::channel();
        cmd_tx
            .send(WorkerCommand::Shutdown { ack: sd_tx })
            .await
            .unwrap();

        assert_eq!(rx1.await.unwrap().unwrap().choices[0].message.content, "1");
        assert_eq!(rx2.await.unwrap().unwrap().choices[0].message.content, "2");
        assert_eq!(rx3.await.unwrap().unwrap().choices[0].message.content, "3");
        sd_rx.await.unwrap();

        handle.join().unwrap();
    }

    #[tokio::test]
    async fn worker_panic_during_generate_poisons_backend() {
        let adapter = ScriptedAdapter::new(vec![ScriptStep::PanicOnSample]);
        let (cmd_tx, poisoned, handle) = spawn_worker(adapter);

        let (load_tx, load_rx) = oneshot::channel();
        cmd_tx
            .send(load_cmd(load_tx))
            .await
            .unwrap();
        load_rx.await.unwrap().unwrap();

        assert!(!poisoned.load(Ordering::SeqCst));

        let (chat_tx, chat_rx) = oneshot::channel();
        let request = crate::api::types::ChatCompletionRequest {
            model: "test-model".into(),
            messages: vec![crate::api::types::ChatMessage {
                role: "user".into(),
                content: "hi".into(),
            }],
            stream: false,
            temperature: 0.7,
            top_p: 0.9,
            max_tokens: 16,
            stop: Vec::new(),
            seed: None,
        };
        cmd_tx
            .send(WorkerCommand::Chat {
                request,
                ack: chat_tx,
            })
            .await
            .unwrap();

        // catch_unwind drops the ack; the chat receiver should see that.
        let result = chat_rx.await;
        assert!(result.is_err(), "ack should have been dropped due to panic");

        handle.join().unwrap();
        assert!(poisoned.load(Ordering::SeqCst));
    }
}
