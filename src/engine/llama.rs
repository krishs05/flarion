use std::num::NonZeroU32;
use std::sync::Mutex;

use tracing::{debug, info, warn};

use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend as LlamaCppBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{AddBos, LlamaChatMessage, LlamaChatTemplate, LlamaModel};
use llama_cpp_2::sampling::LlamaSampler;

use crate::api::types::{
    ChatCompletionChoice, ChatCompletionChunk, ChatCompletionChunkChoice, ChatCompletionRequest,
    ChatCompletionResponse, ChatDelta, ChatMessage, Usage,
};
use crate::config::ModelConfig;
use crate::engine::backend::ModelInfo;
use crate::error::EngineError;

/// Internal state that holds the loaded llama.cpp model and backend.
/// All fields are !Send/!Sync, so access must be serialized via a Mutex.
struct LlamaState {
    backend: LlamaCppBackend,
    model: LlamaModel,
}

/// The inference backend wrapping llama.cpp via the `llama-cpp-2` crate.
///
/// This struct is named `LlamaBackend` to match what `main.rs` expects.
/// The llama-cpp-2 crate's own `LlamaBackend` is imported as `LlamaCppBackend`.
pub struct LlamaBackend {
    config: ModelConfig,
    state: Mutex<Option<LlamaState>>,
}

// SAFETY: All access to the inner !Send/!Sync llama-cpp-2 types is serialized
// through the Mutex. Only one thread can touch the FFI state at a time.
unsafe impl Send for LlamaBackend {}
unsafe impl Sync for LlamaBackend {}

impl LlamaBackend {
    /// Create a new LlamaBackend from a ModelConfig.
    /// The model is NOT loaded yet -- call `load()` to initialize.
    pub fn new(config: &ModelConfig) -> Result<Self, EngineError> {
        Ok(Self {
            config: config.clone(),
            state: Mutex::new(None),
        })
    }
}

/// Format chat messages into a prompt string.
///
/// Attempts to use the model's built-in chat template first. If the model
/// doesn't have one, falls back to a simple ChatML-style format.
fn format_prompt(model: &LlamaModel, messages: &[ChatMessage]) -> Result<String, EngineError> {
    // Try to get the model's chat template
    match model.chat_template(None) {
        Ok(template) => apply_model_template(model, &template, messages),
        Err(_) => {
            debug!("model has no chat template, using ChatML fallback");
            Ok(format_chatml_fallback(messages))
        }
    }
}

/// Apply the model's own chat template via llama.cpp's template engine.
fn apply_model_template(
    model: &LlamaModel,
    template: &LlamaChatTemplate,
    messages: &[ChatMessage],
) -> Result<String, EngineError> {
    let chat_messages: Vec<LlamaChatMessage> = messages
        .iter()
        .map(|m| {
            LlamaChatMessage::new(m.role.clone(), m.content.clone())
                .map_err(|e| EngineError::InferenceFailed(format!("chat message error: {e}")))
        })
        .collect::<Result<Vec<_>, _>>()?;

    model
        .apply_chat_template(template, &chat_messages, true)
        .map_err(|e| EngineError::InferenceFailed(format!("failed to apply chat template: {e}")))
}

/// Simple ChatML fallback template for models that don't ship one.
fn format_chatml_fallback(messages: &[ChatMessage]) -> String {
    let mut prompt = String::new();
    for msg in messages {
        prompt.push_str(&format!(
            "<|im_start|>{}\n{}<|im_end|>\n",
            msg.role, msg.content
        ));
    }
    prompt.push_str("<|im_start|>assistant\n");
    prompt
}

/// Build a sampler chain from request parameters.
fn build_sampler(request: &ChatCompletionRequest, config_seed: Option<u32>) -> LlamaSampler {
    let seed = request.seed.or(config_seed).unwrap_or(0);

    let samplers = vec![
        LlamaSampler::top_p(request.top_p, 1),
        LlamaSampler::temp(request.temperature),
        LlamaSampler::dist(seed),
    ];

    LlamaSampler::chain_simple(samplers)
}

/// Generate a unique completion ID.
fn completion_id() -> String {
    format!("chatcmpl-{}", uuid::Uuid::new_v4())
}

/// Get the current unix timestamp.
fn unix_timestamp() -> i64 {
    chrono::Utc::now().timestamp()
}

#[async_trait::async_trait]
impl crate::engine::backend::InferenceBackend for LlamaBackend {
    async fn load(&self) -> Result<(), EngineError> {
        info!(model_path = %self.config.path.display(), "loading model");

        // Initialize the llama.cpp backend
        let backend = LlamaCppBackend::init().map_err(|e| {
            EngineError::ModelLoadFailed(format!("failed to initialize llama backend: {e}"))
        })?;

        // Configure model params
        let model_params = LlamaModelParams::default().with_n_gpu_layers(self.config.gpu_layers);

        // Load the model from file
        let model = LlamaModel::load_from_file(&backend, &self.config.path, &model_params)
            .map_err(|e| EngineError::ModelLoadFailed(format!("failed to load model file: {e}")))?;

        info!(
            model_id = %self.config.id,
            context_size = self.config.context_size,
            gpu_layers = self.config.gpu_layers,
            "model loaded successfully"
        );

        let mut state = self.state.lock().map_err(|e| {
            EngineError::ModelLoadFailed(format!("failed to acquire state lock: {e}"))
        })?;
        *state = Some(LlamaState { backend, model });

        Ok(())
    }

    async fn chat_completion(
        &self,
        request: ChatCompletionRequest,
    ) -> Result<ChatCompletionResponse, EngineError> {
        // NOTE: We perform synchronous inference directly on the async task because
        // llama-cpp-2 types are !Send and cannot be moved into spawn_blocking.
        // For Phase 1 (single-user), this is acceptable. In Phase 2 this should be
        // improved with a dedicated inference thread and message passing.

        let mut state_guard = self.state.lock().map_err(|e| {
            EngineError::InferenceFailed(format!("failed to acquire state lock: {e}"))
        })?;
        let state = state_guard
            .as_mut()
            .ok_or_else(|| EngineError::InferenceFailed("model not loaded".to_string()))?;

        let id = completion_id();
        let created = unix_timestamp();
        let model_id = self.config.id.clone();

        // Format prompt
        let prompt = format_prompt(&state.model, &request.messages)?;

        // Tokenize
        let tokens = state
            .model
            .str_to_token(&prompt, AddBos::Never)
            .map_err(|e| EngineError::InferenceFailed(format!("tokenization failed: {e}")))?;

        let prompt_token_count = tokens.len() as u32;

        // Check context length
        let context_size = self.config.context_size as usize;
        if tokens.len() > context_size {
            return Err(EngineError::ContextLengthExceeded {
                requested: tokens.len(),
                max: context_size,
            });
        }

        // Create context params
        let batch_size = self.config.batch_size.unwrap_or(512);
        let n_threads = self.config.threads.map_or(4, |t| t as i32);

        let ctx_params = LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(self.config.context_size))
            .with_n_batch(batch_size)
            .with_n_threads(n_threads)
            .with_n_threads_batch(n_threads);

        // Create context
        let mut ctx = state
            .model
            .new_context(&state.backend, ctx_params)
            .map_err(|e| EngineError::InferenceFailed(format!("failed to create context: {e}")))?;

        // Process prompt tokens in batch
        let mut batch = LlamaBatch::new(batch_size as usize, 1);
        batch.add_sequence(&tokens, 0, false).map_err(|e| {
            EngineError::InferenceFailed(format!("failed to add tokens to batch: {e}"))
        })?;

        ctx.decode(&mut batch)
            .map_err(|e| EngineError::InferenceFailed(format!("prompt decode failed: {e}")))?;

        // Build sampler
        let mut sampler = build_sampler(&request, self.config.seed);

        // Generation loop
        let max_tokens = request.max_tokens as usize;
        let mut generated_text = String::new();
        let mut completion_tokens: u32 = 0;
        let mut finish_reason = "length".to_string();
        let mut decoder = encoding_rs::UTF_8.new_decoder();

        for _ in 0..max_tokens {
            // Sample the next token
            let token = sampler.sample(&ctx, -1);
            sampler.accept(token);

            // Check for end of generation
            if state.model.is_eog_token(token) {
                finish_reason = "stop".to_string();
                break;
            }

            // Decode token to text
            match state.model.token_to_piece(token, &mut decoder, false, None) {
                Ok(piece) => {
                    // Check for stop sequences
                    generated_text.push_str(&piece);

                    if let Some(stop_reason) = check_stop_sequences(&generated_text, &request.stop)
                    {
                        // Trim the stop sequence from the output
                        generated_text.truncate(generated_text.len() - stop_reason.len());
                        finish_reason = "stop".to_string();
                        completion_tokens += 1;
                        break;
                    }
                }
                Err(e) => {
                    warn!(error = %e, "failed to decode token, skipping");
                }
            }

            completion_tokens += 1;

            // Prepare next batch with the sampled token
            batch.clear();
            let pos = (tokens.len() + completion_tokens as usize - 1) as i32;
            batch.add(token, pos, &[0], true).map_err(|e| {
                EngineError::InferenceFailed(format!("failed to add token to batch: {e}"))
            })?;

            ctx.decode(&mut batch)
                .map_err(|e| EngineError::InferenceFailed(format!("decode step failed: {e}")))?;
        }

        let total_tokens = prompt_token_count + completion_tokens;

        Ok(ChatCompletionResponse {
            id,
            object: "chat.completion".to_string(),
            created,
            model: model_id,
            choices: vec![ChatCompletionChoice {
                index: 0,
                message: ChatMessage {
                    role: "assistant".to_string(),
                    content: generated_text,
                },
                finish_reason,
            }],
            usage: Usage {
                prompt_tokens: prompt_token_count,
                completion_tokens,
                total_tokens,
            },
        })
    }

    async fn chat_completion_stream(
        &self,
        request: ChatCompletionRequest,
        tx: tokio::sync::mpsc::Sender<ChatCompletionChunk>,
    ) -> Result<(), EngineError> {
        // NOTE: llama-cpp-2 types are !Send, so we cannot hold them across .await
        // points. We collect all chunks synchronously during inference, then send
        // them after the !Send types are dropped. This means the "streaming" response
        // is actually batched -- true token-by-token streaming requires a dedicated
        // inference thread (Phase 2 improvement).

        let chunks = {
            let mut state_guard = self.state.lock().map_err(|e| {
                EngineError::InferenceFailed(format!("failed to acquire state lock: {e}"))
            })?;
            let state = state_guard
                .as_mut()
                .ok_or_else(|| EngineError::InferenceFailed("model not loaded".to_string()))?;

            let id = completion_id();
            let created = unix_timestamp();
            let model_id = self.config.id.clone();

            // Format prompt
            let prompt = format_prompt(&state.model, &request.messages)?;

            // Tokenize
            let tokens = state
                .model
                .str_to_token(&prompt, AddBos::Never)
                .map_err(|e| EngineError::InferenceFailed(format!("tokenization failed: {e}")))?;

            // Check context length
            let context_size = self.config.context_size as usize;
            if tokens.len() > context_size {
                return Err(EngineError::ContextLengthExceeded {
                    requested: tokens.len(),
                    max: context_size,
                });
            }

            // Create context params
            let batch_size = self.config.batch_size.unwrap_or(512);
            let n_threads = self.config.threads.map_or(4, |t| t as i32);

            let ctx_params = LlamaContextParams::default()
                .with_n_ctx(NonZeroU32::new(self.config.context_size))
                .with_n_batch(batch_size)
                .with_n_threads(n_threads)
                .with_n_threads_batch(n_threads);

            // Create context
            let mut ctx = state
                .model
                .new_context(&state.backend, ctx_params)
                .map_err(|e| {
                    EngineError::InferenceFailed(format!("failed to create context: {e}"))
                })?;

            // Process prompt tokens
            let mut batch = LlamaBatch::new(batch_size as usize, 1);
            batch.add_sequence(&tokens, 0, false).map_err(|e| {
                EngineError::InferenceFailed(format!("failed to add tokens to batch: {e}"))
            })?;

            ctx.decode(&mut batch)
                .map_err(|e| EngineError::InferenceFailed(format!("prompt decode failed: {e}")))?;

            // Collect all chunks during synchronous inference
            let mut chunks = Vec::new();

            // Initial chunk with role
            chunks.push(ChatCompletionChunk {
                id: id.clone(),
                object: "chat.completion.chunk".to_string(),
                created,
                model: model_id.clone(),
                choices: vec![ChatCompletionChunkChoice {
                    index: 0,
                    delta: ChatDelta {
                        role: Some("assistant".to_string()),
                        content: None,
                    },
                    finish_reason: None,
                }],
            });

            // Build sampler
            let mut sampler = build_sampler(&request, self.config.seed);

            // Generation loop
            let max_tokens = request.max_tokens as usize;
            let mut generated_text = String::new();
            let mut completion_tokens: u32 = 0;
            let mut decoder = encoding_rs::UTF_8.new_decoder();

            for _ in 0..max_tokens {
                let token = sampler.sample(&ctx, -1);
                sampler.accept(token);

                if state.model.is_eog_token(token) {
                    chunks.push(ChatCompletionChunk {
                        id: id.clone(),
                        object: "chat.completion.chunk".to_string(),
                        created,
                        model: model_id.clone(),
                        choices: vec![ChatCompletionChunkChoice {
                            index: 0,
                            delta: ChatDelta {
                                role: None,
                                content: None,
                            },
                            finish_reason: Some("stop".to_string()),
                        }],
                    });
                    break;
                }

                match state.model.token_to_piece(token, &mut decoder, false, None) {
                    Ok(piece) => {
                        generated_text.push_str(&piece);

                        // Check stop sequences
                        if let Some(stop_seq) = check_stop_sequences(&generated_text, &request.stop)
                        {
                            let trimmed_piece_len = piece.len().saturating_sub(stop_seq.len());
                            if trimmed_piece_len > 0 {
                                chunks.push(ChatCompletionChunk {
                                    id: id.clone(),
                                    object: "chat.completion.chunk".to_string(),
                                    created,
                                    model: model_id.clone(),
                                    choices: vec![ChatCompletionChunkChoice {
                                        index: 0,
                                        delta: ChatDelta {
                                            role: None,
                                            content: Some(piece[..trimmed_piece_len].to_string()),
                                        },
                                        finish_reason: None,
                                    }],
                                });
                            }

                            chunks.push(ChatCompletionChunk {
                                id: id.clone(),
                                object: "chat.completion.chunk".to_string(),
                                created,
                                model: model_id.clone(),
                                choices: vec![ChatCompletionChunkChoice {
                                    index: 0,
                                    delta: ChatDelta {
                                        role: None,
                                        content: None,
                                    },
                                    finish_reason: Some("stop".to_string()),
                                }],
                            });
                            break;
                        }

                        // Content chunk
                        chunks.push(ChatCompletionChunk {
                            id: id.clone(),
                            object: "chat.completion.chunk".to_string(),
                            created,
                            model: model_id.clone(),
                            choices: vec![ChatCompletionChunkChoice {
                                index: 0,
                                delta: ChatDelta {
                                    role: None,
                                    content: Some(piece),
                                },
                                finish_reason: None,
                            }],
                        });
                    }
                    Err(e) => {
                        warn!(error = %e, "failed to decode token in stream, skipping");
                    }
                }

                completion_tokens += 1;

                // Prepare next batch
                batch.clear();
                let pos = (tokens.len() + completion_tokens as usize - 1) as i32;
                batch.add(token, pos, &[0], true).map_err(|e| {
                    EngineError::InferenceFailed(format!("failed to add token to batch: {e}"))
                })?;

                ctx.decode(&mut batch).map_err(|e| {
                    EngineError::InferenceFailed(format!("decode step failed: {e}"))
                })?;
            }

            // If loop ended without a stop/eog, add "length" finish chunk
            let has_finish = chunks.iter().any(|c| {
                c.choices
                    .first()
                    .and_then(|ch| ch.finish_reason.as_ref())
                    .is_some()
            });
            if !has_finish {
                chunks.push(ChatCompletionChunk {
                    id,
                    object: "chat.completion.chunk".to_string(),
                    created,
                    model: model_id,
                    choices: vec![ChatCompletionChunkChoice {
                        index: 0,
                        delta: ChatDelta {
                            role: None,
                            content: None,
                        },
                        finish_reason: Some("length".to_string()),
                    }],
                });
            }

            chunks
            // state_guard, ctx, batch, sampler, decoder are all dropped here
        };

        // Now send all collected chunks -- no !Send types are held
        for chunk in chunks {
            if tx.send(chunk).await.is_err() {
                return Ok(()); // Client disconnected
            }
        }

        Ok(())
    }

    fn model_info(&self) -> ModelInfo {
        let loaded = self.state.lock().map(|s| s.is_some()).unwrap_or(false);

        ModelInfo {
            id: self.config.id.clone(),
            context_size: self.config.context_size,
            loaded,
        }
    }
}

/// Check if the generated text ends with any of the stop sequences.
/// Returns the matching stop sequence if found.
fn check_stop_sequences<'a>(text: &str, stop_sequences: &'a [String]) -> Option<&'a str> {
    for stop in stop_sequences {
        if text.ends_with(stop.as_str()) {
            return Some(stop.as_str());
        }
    }
    None
}
