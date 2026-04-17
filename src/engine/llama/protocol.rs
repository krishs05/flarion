use tokio::sync::{mpsc, oneshot};

use crate::api::types::{ChatCompletionChunk, ChatCompletionRequest, ChatCompletionResponse};
use crate::error::EngineError;

/// Commands sent from `LlamaBackend` (async side) to the worker OS thread.
pub(super) enum WorkerCommand {
    /// Load the GGUF model into memory. Sent exactly once, as the first
    /// command after the worker thread spawns.
    Load {
        ack: oneshot::Sender<Result<(), EngineError>>,
    },

    /// Non-streaming chat completion. Worker sends the full response via `ack`.
    Chat {
        request: ChatCompletionRequest,
        ack: oneshot::Sender<Result<ChatCompletionResponse, EngineError>>,
    },

    /// Streaming chat completion. Worker sends chunks to `chunks` as they
    /// are generated and observes `chunks.is_closed()` between tokens to
    /// cancel generation when the client disconnects. Final Ok/Err arrives
    /// via `done` regardless of cancellation.
    ChatStream {
        request: ChatCompletionRequest,
        chunks: mpsc::Sender<ChatCompletionChunk>,
        done: oneshot::Sender<Result<(), EngineError>>,
    },

    /// Graceful shutdown. Worker finishes any in-progress inference, drops
    /// the model state, and exits the thread. Sent by
    /// `LlamaBackend::shutdown`.
    Shutdown { ack: oneshot::Sender<()> },
}

/// What the dispatch loop should do after handling one command.
pub(super) enum DispatchOutcome {
    Continue,
    Shutdown,
}
