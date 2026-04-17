use std::num::NonZeroU32;
use std::ops::ControlFlow;
use std::path::Path;

use tracing::{debug, info, warn};

use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend as LlamaCppBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{AddBos, LlamaChatMessage, LlamaChatTemplate, LlamaModel};
use llama_cpp_2::openai::OpenAIChatTemplateParams;
use llama_cpp_2::sampling::LlamaSampler;

use crate::api::types::{ChatCompletionRequest, ChatMessage};
use crate::config::ModelConfig;
use crate::error::EngineError;

/// Format chat messages into a prompt using the model's own chat template,
/// falling back through OAI-compat Jinja and a hand-rolled ChatML template
/// for models whose embedded template the legacy engine can't handle.
pub(super) fn format_prompt(
    model: &LlamaModel,
    messages: &[ChatMessage],
) -> Result<String, EngineError> {
    match model.chat_template(None) {
        Ok(template) => match apply_model_template(model, &template, messages) {
            Ok(s) => Ok(s),
            Err(e) => {
                warn!(error = %e, "legacy chat template failed, trying Jinja OAI-compat path");
                try_oaicompat_or_fallback(model, &template, messages)
            }
        },
        Err(_) => {
            debug!("model has no chat template, using ChatML fallback");
            Ok(format_chatml_fallback(messages))
        }
    }
}

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

fn try_oaicompat_or_fallback(
    model: &LlamaModel,
    template: &LlamaChatTemplate,
    messages: &[ChatMessage],
) -> Result<String, EngineError> {
    let messages_json = serde_json::to_string(messages).map_err(|e| {
        EngineError::InferenceFailed(format!("serialize messages for chat template: {e}"))
    })?;
    let params = OpenAIChatTemplateParams {
        messages_json: &messages_json,
        tools_json: None,
        tool_choice: None,
        json_schema: None,
        grammar: None,
        reasoning_format: None,
        chat_template_kwargs: None,
        add_generation_prompt: true,
        use_jinja: true,
        parallel_tool_calls: false,
        enable_thinking: false,
        add_bos: false,
        add_eos: false,
        parse_tool_calls: false,
    };
    match model.apply_chat_template_oaicompat(template, &params) {
        Ok(result) => Ok(result.prompt),
        Err(e) => {
            warn!(error = %e, "OAI-compat chat template failed, using Gemma-style turn fallback");
            Ok(format_gemma_turn_fallback(messages))
        }
    }
}

fn format_gemma_turn_fallback(messages: &[ChatMessage]) -> String {
    let mut prompt = String::new();
    for msg in messages {
        let role = match msg.role.as_str() {
            "user" => "user",
            "assistant" => "model",
            "system" => "user",
            _ => "user",
        };
        prompt.push_str("<start_of_turn>");
        prompt.push_str(role);
        prompt.push('\n');
        prompt.push_str(&msg.content);
        prompt.push_str("<end_of_turn>\n");
    }
    prompt.push_str("<start_of_turn>model\n");
    prompt
}

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

pub(super) fn build_sampler(
    request: &ChatCompletionRequest,
    config_seed: Option<u32>,
) -> LlamaSampler {
    let seed = request.seed.or(config_seed).unwrap_or(0);

    let samplers = vec![
        LlamaSampler::top_p(request.top_p, 1),
        LlamaSampler::temp(request.temperature),
        LlamaSampler::dist(seed),
    ];

    LlamaSampler::chain_simple(samplers)
}

pub(super) fn completion_id() -> String {
    format!("chatcmpl-{}", uuid::Uuid::new_v4())
}

pub(super) fn unix_timestamp() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Return the first stop sequence from `stop_sequences` that `text` ends with.
pub(super) fn check_stop_sequences<'a>(
    text: &str,
    stop_sequences: &'a [String],
) -> Option<&'a str> {
    for stop in stop_sequences {
        if text.ends_with(stop.as_str()) {
            return Some(stop.as_str());
        }
    }
    None
}

/// Event emitted by `ModelAdapter::generate` to its callback.
pub(super) enum GenerationEvent<'a> {
    Token { text: &'a str },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FinishReason {
    Stop,
    Length,
}

impl FinishReason {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            FinishReason::Stop => "stop",
            FinishReason::Length => "length",
        }
    }
}

pub(super) struct GenerationResult {
    pub(super) text: String,
    pub(super) prompt_tokens: u32,
    pub(super) completion_tokens: u32,
    pub(super) finish_reason: FinishReason,
    /// Set when the callback returned `ControlFlow::Break` before EOS/length.
    pub(super) canceled: bool,
}

/// Abstraction over the llama-cpp-2 calls the worker makes; swapped for
/// `ScriptedAdapter` in tests.
pub(super) trait ModelAdapter: Send + 'static {
    fn load(&mut self, config: &ModelConfig) -> Result<(), EngineError>;

    /// Generate tokens, invoking `on_event` for each sampled token. If the
    /// callback returns `ControlFlow::Break`, the run ends early with
    /// `GenerationResult::canceled = true` and partial `text` /
    /// `completion_tokens` reflecting what was produced so far.
    fn generate(
        &mut self,
        request: &ChatCompletionRequest,
        config: &ModelConfig,
        on_event: &mut dyn FnMut(GenerationEvent<'_>) -> ControlFlow<()>,
    ) -> Result<GenerationResult, EngineError>;
}

struct LlamaState {
    backend: LlamaCppBackend,
    model: LlamaModel,
}

#[derive(Default)]
pub(super) struct LlamaAdapter {
    state: Option<LlamaState>,
}

impl ModelAdapter for LlamaAdapter {
    fn load(&mut self, config: &ModelConfig) -> Result<(), EngineError> {
        let path: &Path = config.path.as_ref().ok_or_else(|| {
            EngineError::ModelLoadFailed(format!("local backend '{}' has no path", config.id))
        })?;

        info!(model_path = %path.display(), "loading model");

        let backend = LlamaCppBackend::init().map_err(|e| {
            EngineError::ModelLoadFailed(format!("failed to init llama backend: {e}"))
        })?;

        let params = LlamaModelParams::default().with_n_gpu_layers(config.gpu_layers);
        let model = LlamaModel::load_from_file(&backend, path, &params)
            .map_err(|e| EngineError::ModelLoadFailed(format!("failed to load model: {e}")))?;

        info!(
            model_id = %config.id,
            context_size = config.context_size,
            gpu_layers = config.gpu_layers,
            "model loaded successfully"
        );

        self.state = Some(LlamaState { backend, model });
        Ok(())
    }

    fn generate(
        &mut self,
        request: &ChatCompletionRequest,
        config: &ModelConfig,
        on_event: &mut dyn FnMut(GenerationEvent<'_>) -> ControlFlow<()>,
    ) -> Result<GenerationResult, EngineError> {
        let state = self
            .state
            .as_mut()
            .ok_or_else(|| EngineError::InferenceFailed("model not loaded".into()))?;

        let prompt = format_prompt(&state.model, &request.messages)?;
        let tokens = state
            .model
            .str_to_token(&prompt, AddBos::Never)
            .map_err(|e| EngineError::InferenceFailed(format!("tokenization failed: {e}")))?;

        let prompt_token_count = tokens.len() as u32;

        let context_size = config.context_size as usize;
        if tokens.len() > context_size {
            return Err(EngineError::ContextLengthExceeded {
                requested: tokens.len(),
                max: context_size,
            });
        }

        let batch_size = config.batch_size.unwrap_or(512);
        let n_threads = config.threads.map_or(4, |t| t as i32);

        let ctx_params = LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(config.context_size))
            .with_n_batch(batch_size)
            .with_n_threads(n_threads)
            .with_n_threads_batch(n_threads);

        let mut ctx = state
            .model
            .new_context(&state.backend, ctx_params)
            .map_err(|e| EngineError::InferenceFailed(format!("context creation failed: {e}")))?;

        let mut batch = LlamaBatch::new(batch_size as usize, 1);
        batch.add_sequence(&tokens, 0, false).map_err(|e| {
            EngineError::InferenceFailed(format!("failed to add prompt tokens: {e}"))
        })?;

        ctx.decode(&mut batch)
            .map_err(|e| EngineError::InferenceFailed(format!("prompt decode failed: {e}")))?;

        let mut sampler = build_sampler(request, config.seed);

        let max_tokens = request.max_tokens as usize;
        let mut generated_text = String::new();
        let mut completion_tokens: u32 = 0;
        let mut finish_reason = FinishReason::Length;
        let mut canceled = false;
        let mut decoder = encoding_rs::UTF_8.new_decoder();

        for _ in 0..max_tokens {
            let token = sampler.sample(&ctx, -1);
            sampler.accept(token);

            if state.model.is_eog_token(token) {
                finish_reason = FinishReason::Stop;
                break;
            }

            match state.model.token_to_piece(token, &mut decoder, false, None) {
                Ok(piece) => {
                    generated_text.push_str(&piece);

                    if let Some(stop_seq) = check_stop_sequences(&generated_text, &request.stop) {
                        generated_text.truncate(generated_text.len() - stop_seq.len());
                        finish_reason = FinishReason::Stop;
                        completion_tokens += 1;
                        break;
                    }

                    completion_tokens += 1;

                    if matches!(
                        on_event(GenerationEvent::Token { text: &piece }),
                        ControlFlow::Break(())
                    ) {
                        canceled = true;
                        break;
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "failed to decode token, skipping");
                    completion_tokens += 1;
                }
            }

            batch.clear();
            let pos = (tokens.len() + completion_tokens as usize - 1) as i32;
            batch.add(token, pos, &[0], true).map_err(|e| {
                EngineError::InferenceFailed(format!("failed to add token to batch: {e}"))
            })?;

            ctx.decode(&mut batch)
                .map_err(|e| EngineError::InferenceFailed(format!("decode step failed: {e}")))?;
        }

        Ok(GenerationResult {
            text: generated_text,
            prompt_tokens: prompt_token_count,
            completion_tokens,
            finish_reason,
            canceled,
        })
    }
}

#[cfg(test)]
pub(super) mod test_adapter {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Debug)]
    #[allow(dead_code)]
    pub(crate) enum ScriptStep {
        Token(String),
        Eog,
        LoadFails(String),
        PanicOnSample,
        /// Sleep inside `generate` to simulate slow inference.
        SleepMs(u64),
    }

    pub(crate) struct ScriptedAdapter {
        pub(crate) script: Arc<Mutex<Vec<ScriptStep>>>,
        /// Number of token steps consumed, used by tests to assert early cancel.
        pub(crate) sampled: Arc<std::sync::atomic::AtomicUsize>,
        loaded: bool,
    }

    impl ScriptedAdapter {
        pub(crate) fn new(script: Vec<ScriptStep>) -> Self {
            Self {
                script: Arc::new(Mutex::new(script)),
                sampled: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
                loaded: false,
            }
        }

        pub(crate) fn sampled_count(&self) -> usize {
            self.sampled.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    impl ModelAdapter for ScriptedAdapter {
        fn load(&mut self, _config: &ModelConfig) -> Result<(), EngineError> {
            let mut script = self.script.lock().unwrap();
            if let Some(ScriptStep::LoadFails(msg)) = script.first().cloned() {
                script.remove(0);
                return Err(EngineError::ModelLoadFailed(msg));
            }
            self.loaded = true;
            Ok(())
        }

        fn generate(
            &mut self,
            _request: &ChatCompletionRequest,
            _config: &ModelConfig,
            on_event: &mut dyn FnMut(GenerationEvent<'_>) -> ControlFlow<()>,
        ) -> Result<GenerationResult, EngineError> {
            if !self.loaded {
                return Err(EngineError::InferenceFailed("not loaded".into()));
            }
            let mut text = String::new();
            let mut completion_tokens = 0u32;
            let mut finish = FinishReason::Length;
            let mut canceled = false;

            loop {
                let step = {
                    let mut script = self.script.lock().unwrap();
                    if script.is_empty() {
                        break;
                    }
                    script.remove(0)
                };
                match step {
                    ScriptStep::Token(t) => {
                        self.sampled
                            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        text.push_str(&t);
                        completion_tokens += 1;
                        if matches!(
                            on_event(GenerationEvent::Token { text: &t }),
                            ControlFlow::Break(())
                        ) {
                            canceled = true;
                            break;
                        }
                    }
                    ScriptStep::Eog => {
                        finish = FinishReason::Stop;
                        break;
                    }
                    ScriptStep::PanicOnSample => {
                        panic!("ScriptedAdapter: PanicOnSample step reached");
                    }
                    ScriptStep::SleepMs(ms) => {
                        std::thread::sleep(std::time::Duration::from_millis(ms));
                    }
                    // LoadFails only applies at load time.
                    ScriptStep::LoadFails(_) => {}
                }
            }

            Ok(GenerationResult {
                text,
                prompt_tokens: 1,
                completion_tokens,
                finish_reason: finish,
                canceled,
            })
        }
    }
}

#[cfg(test)]
mod scripted_adapter_tests {
    use super::test_adapter::{ScriptStep, ScriptedAdapter};
    use super::*;
    use crate::api::types::ChatMessage;
    use crate::config::BackendType;
    use std::ops::ControlFlow;

    fn minimal_req() -> ChatCompletionRequest {
        ChatCompletionRequest {
            model: "test".into(),
            messages: vec![ChatMessage {
                role: "user".into(),
                content: "hi".into(),
            }],
            stream: false,
            temperature: 0.7,
            top_p: 0.9,
            max_tokens: 16,
            stop: Vec::new(),
            seed: None,
        }
    }

    fn minimal_cfg() -> ModelConfig {
        ModelConfig {
            id: "test".into(),
            backend: BackendType::Local,
            path: Some(std::path::PathBuf::from("/tmp/test.gguf")),
            context_size: 4096,
            gpu_layers: 99,
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
        }
    }

    #[test]
    fn scripted_load_succeeds_by_default() {
        let mut a = ScriptedAdapter::new(vec![]);
        assert!(a.load(&minimal_cfg()).is_ok());
    }

    #[test]
    fn scripted_load_fails_when_step_requests() {
        let mut a = ScriptedAdapter::new(vec![ScriptStep::LoadFails("boom".into())]);
        let err = a.load(&minimal_cfg()).unwrap_err();
        assert!(matches!(err, EngineError::ModelLoadFailed(ref m) if m == "boom"));
    }

    #[test]
    fn scripted_generate_emits_tokens_and_stops() {
        let mut a = ScriptedAdapter::new(vec![
            ScriptStep::Token("hi".into()),
            ScriptStep::Token(" ".into()),
            ScriptStep::Token("world".into()),
            ScriptStep::Eog,
        ]);
        a.load(&minimal_cfg()).unwrap();
        let mut events = Vec::new();
        let result = a
            .generate(&minimal_req(), &minimal_cfg(), &mut |e| {
                let GenerationEvent::Token { text } = e;
                events.push(text.to_string());
                ControlFlow::Continue(())
            })
            .unwrap();
        assert_eq!(events, vec!["hi", " ", "world"]);
        assert_eq!(result.text, "hi world");
        assert_eq!(result.completion_tokens, 3);
        assert_eq!(result.finish_reason, FinishReason::Stop);
        assert!(!result.canceled);
    }

    #[test]
    fn scripted_generate_cancels_on_break() {
        let mut a = ScriptedAdapter::new(vec![
            ScriptStep::Token("a".into()),
            ScriptStep::Token("b".into()),
            ScriptStep::Token("c".into()),
            ScriptStep::Eog,
        ]);
        a.load(&minimal_cfg()).unwrap();
        let mut count = 0;
        let result = a
            .generate(&minimal_req(), &minimal_cfg(), &mut |_| {
                count += 1;
                if count >= 2 {
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            })
            .unwrap();
        assert!(result.canceled);
        assert_eq!(result.completion_tokens, 2);
        assert_eq!(a.sampled_count(), 2);
    }

    #[test]
    #[should_panic(expected = "PanicOnSample")]
    fn scripted_generate_panics_on_script_step() {
        let mut a = ScriptedAdapter::new(vec![ScriptStep::PanicOnSample]);
        a.load(&minimal_cfg()).unwrap();
        let _ = a.generate(&minimal_req(), &minimal_cfg(), &mut |_| {
            ControlFlow::Continue(())
        });
    }
}
