use std::time::Duration;

use eventsource_stream::Eventsource;
use futures_util::StreamExt;
use serde_json::{Value, json};

use crate::api::types::{ChatCompletionChunk, ChatCompletionRequest, ChatCompletionResponse};
use crate::config::ModelConfig;
use crate::engine::backend::{InferenceBackend, ModelInfo};
use crate::error::EngineError;

const OPENAI_DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";
const GROQ_DEFAULT_BASE_URL: &str = "https://api.groq.com/openai/v1";

pub struct OpenAICompatibleBackend {
    id: String,
    upstream_model: String,
    provider_name: &'static str,
    base_url: String,
    api_key: String,
    client: reqwest::Client,
    max_tokens_cap: u32,
}

impl OpenAICompatibleBackend {
    /// `provider` must be one of "openai" or "groq".
    pub fn new(cfg: &ModelConfig, provider: &'static str) -> Result<Self, EngineError> {
        let api_key = cfg.api_key.clone().ok_or_else(|| {
            EngineError::ModelLoadFailed(format!(
                "model '{}': api_key required for {} backend",
                cfg.id, provider
            ))
        })?;

        let upstream_model = cfg.upstream_model.clone().unwrap_or_else(|| cfg.id.clone());

        let base_url = cfg.base_url.clone().unwrap_or_else(|| match provider {
            "openai" => OPENAI_DEFAULT_BASE_URL.to_string(),
            "groq" => GROQ_DEFAULT_BASE_URL.to_string(),
            other => panic!("OpenAICompatibleBackend constructed with unknown provider: {other}"),
        });

        let timeout = Duration::from_secs(cfg.timeout_secs.unwrap_or(300));
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| {
                EngineError::ModelLoadFailed(format!("reqwest client build failed: {e}"))
            })?;

        let max_tokens_cap = cfg.max_tokens_cap.unwrap_or(8192);

        Ok(Self {
            id: cfg.id.clone(),
            upstream_model,
            provider_name: provider,
            base_url,
            api_key,
            client,
            max_tokens_cap,
        })
    }

    fn endpoint(&self) -> String {
        format!("{}/chat/completions", self.base_url.trim_end_matches('/'))
    }

    fn build_body(&self, request: &ChatCompletionRequest, stream: bool) -> Value {
        let mut body = json!({
            "model": self.upstream_model,
            "messages": request.messages.iter().map(|m| json!({
                "role": m.role,
                "content": m.content,
            })).collect::<Vec<_>>(),
            "temperature": request.temperature,
            "top_p": request.top_p,
            "max_tokens": request.max_tokens,
            "stream": stream,
        });
        if !request.stop.is_empty() {
            body["stop"] = json!(request.stop);
        }
        if let Some(seed) = request.seed {
            body["seed"] = json!(seed);
        }
        body
    }

    fn upstream_error(&self, status: reqwest::StatusCode, body: &str) -> EngineError {
        let upstream_message = serde_json::from_str::<Value>(body)
            .ok()
            .and_then(|v| {
                v.get("error")
                    .and_then(|e| e.get("message"))
                    .and_then(|m| m.as_str())
                    .map(String::from)
            })
            .unwrap_or_else(|| body.chars().take(200).collect::<String>());

        let full = format!(
            "upstream {} {}: {}",
            self.provider_name, status, upstream_message
        );

        if status.as_u16() == 429 {
            EngineError::RateLimited { retry_after: None }
        } else if status.is_server_error() {
            EngineError::UpstreamServerError {
                status: status.as_u16(),
                body: full,
            }
        } else {
            EngineError::InferenceFailed(full)
        }
    }

    fn request_error(&self, err: reqwest::Error) -> EngineError {
        if err.is_timeout() {
            EngineError::Timeout
        } else if err.is_connect() || err.is_request() {
            EngineError::Network(err.to_string())
        } else {
            EngineError::InferenceFailed(format!(
                "upstream {} request error: {err}",
                self.provider_name
            ))
        }
    }
}

#[async_trait::async_trait]
impl InferenceBackend for OpenAICompatibleBackend {
    async fn load(&self) -> Result<(), EngineError> {
        Ok(())
    }

    async fn chat_completion(
        &self,
        request: ChatCompletionRequest,
    ) -> Result<ChatCompletionResponse, EngineError> {
        let body = self.build_body(&request, false);
        let response = self
            .client
            .post(self.endpoint())
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| self.request_error(e))?;

        if response.status().as_u16() == 429 {
            let retry_after = response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok())
                .map(std::time::Duration::from_secs);
            return Err(EngineError::RateLimited { retry_after });
        }

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(self.upstream_error(status, &text));
        }

        let mut parsed: ChatCompletionResponse = response.json().await.map_err(|e| {
            EngineError::InferenceFailed(format!("failed to parse upstream response: {e}"))
        })?;
        // Rewrite upstream id to the local id before returning to the client.
        parsed.model = self.id.clone();
        Ok(parsed)
    }

    async fn chat_completion_stream(
        &self,
        request: ChatCompletionRequest,
        tx: tokio::sync::mpsc::Sender<ChatCompletionChunk>,
    ) -> Result<(), EngineError> {
        let body = self.build_body(&request, true);
        let response = self
            .client
            .post(self.endpoint())
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| self.request_error(e))?;

        if response.status().as_u16() == 429 {
            let retry_after = response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok())
                .map(std::time::Duration::from_secs);
            return Err(EngineError::RateLimited { retry_after });
        }

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(self.upstream_error(status, &text));
        }

        let mut events = response.bytes_stream().eventsource();
        while let Some(event) = events.next().await {
            let event = match event {
                Ok(e) => e,
                Err(e) => {
                    tracing::warn!(error = %e, "upstream SSE stream error");
                    break;
                }
            };

            if event.data == "[DONE]" {
                break;
            }

            match serde_json::from_str::<ChatCompletionChunk>(&event.data) {
                Ok(mut chunk) => {
                    chunk.model = self.id.clone();
                    if tx.send(chunk).await.is_err() {
                        break;
                    }
                }
                Err(e) => {
                    // Never log `event.data` — it may contain prompt echoes
                    // or user content. Length is enough for triage.
                    tracing::warn!(
                        error = %e,
                        data_len = event.data.len(),
                        "failed to parse upstream chunk"
                    );
                }
            }
        }

        Ok(())
    }

    fn model_info(&self) -> ModelInfo {
        ModelInfo {
            id: self.id.clone(),
            context_size: 0,
            loaded: true,
            provider: self.provider_name.to_string(),
        }
    }

    fn provider(&self) -> &'static str {
        self.provider_name
    }

    fn max_tokens_cap(&self) -> u32 {
        self.max_tokens_cap
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::types::ChatMessage;

    fn cfg(
        id: &str,
        backend_str: crate::config::BackendType,
        api_key: Option<&str>,
    ) -> ModelConfig {
        ModelConfig {
            id: id.to_string(),
            backend: backend_str,
            path: None,
            context_size: 4096,
            gpu_layers: 99,
            threads: None,
            batch_size: None,
            seed: None,
            api_key: api_key.map(String::from),
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

    #[test]
    fn test_openai_default_base_url() {
        let backend = OpenAICompatibleBackend::new(
            &cfg("gpt-4o", crate::config::BackendType::Openai, Some("sk-x")),
            "openai",
        )
        .unwrap();
        assert_eq!(backend.base_url, OPENAI_DEFAULT_BASE_URL);
    }

    #[test]
    fn test_groq_default_base_url() {
        let backend = OpenAICompatibleBackend::new(
            &cfg("llama-x", crate::config::BackendType::Groq, Some("gsk-x")),
            "groq",
        )
        .unwrap();
        assert_eq!(backend.base_url, GROQ_DEFAULT_BASE_URL);
    }

    #[test]
    fn test_explicit_base_url_wins() {
        let mut c = cfg("gpt-4o", crate::config::BackendType::Openai, Some("sk-x"));
        c.base_url = Some("https://my-proxy.test/v1".into());
        let backend = OpenAICompatibleBackend::new(&c, "openai").unwrap();
        assert_eq!(backend.base_url, "https://my-proxy.test/v1");
    }

    #[test]
    fn test_upstream_model_defaults_to_id() {
        let backend = OpenAICompatibleBackend::new(
            &cfg("gpt-4o", crate::config::BackendType::Openai, Some("sk-x")),
            "openai",
        )
        .unwrap();
        assert_eq!(backend.upstream_model, "gpt-4o");
    }

    #[test]
    fn test_explicit_upstream_model_used() {
        let mut c = cfg("local-id", crate::config::BackendType::Groq, Some("gsk-x"));
        c.upstream_model = Some("llama-3.3-70b-versatile".into());
        let backend = OpenAICompatibleBackend::new(&c, "groq").unwrap();
        assert_eq!(backend.upstream_model, "llama-3.3-70b-versatile");
    }

    #[test]
    fn test_missing_api_key_errors() {
        let result = OpenAICompatibleBackend::new(
            &cfg("gpt-4o", crate::config::BackendType::Openai, None),
            "openai",
        );
        assert!(matches!(result, Err(EngineError::ModelLoadFailed(_))));
    }

    #[test]
    fn test_endpoint_handles_trailing_slash() {
        let mut c = cfg("gpt-4o", crate::config::BackendType::Openai, Some("sk-x"));
        c.base_url = Some("https://api.openai.com/v1/".into());
        let backend = OpenAICompatibleBackend::new(&c, "openai").unwrap();
        assert_eq!(
            backend.endpoint(),
            "https://api.openai.com/v1/chat/completions"
        );
    }

    #[test]
    fn test_provider_field() {
        let backend = OpenAICompatibleBackend::new(
            &cfg("gpt-4o", crate::config::BackendType::Openai, Some("sk-x")),
            "openai",
        )
        .unwrap();
        assert_eq!(backend.provider(), "openai");
        assert_eq!(backend.model_info().provider, "openai");
    }

    #[test]
    fn test_build_body_basic() {
        let backend = OpenAICompatibleBackend::new(
            &cfg("gpt-4o", crate::config::BackendType::Openai, Some("sk-x")),
            "openai",
        )
        .unwrap();
        let req = ChatCompletionRequest {
            model: "gpt-4o".into(),
            messages: vec![ChatMessage {
                role: "user".into(),
                content: "hi".into(),
            }],
            stream: false,
            temperature: 0.7,
            top_p: 0.9,
            max_tokens: 100,
            stop: vec![],
            seed: None,
        };
        let body = backend.build_body(&req, false);
        assert_eq!(body["model"], "gpt-4o");
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["messages"][0]["content"], "hi");
        assert_eq!(body["stream"], false);
        assert!(body.get("stop").is_none(), "empty stop should not be sent");
        assert!(body.get("seed").is_none(), "absent seed should not be sent");
    }

    #[test]
    fn test_build_body_includes_optional_fields() {
        let backend = OpenAICompatibleBackend::new(
            &cfg("gpt-4o", crate::config::BackendType::Openai, Some("sk-x")),
            "openai",
        )
        .unwrap();
        let req = ChatCompletionRequest {
            model: "gpt-4o".into(),
            messages: vec![],
            stream: true,
            temperature: 0.5,
            top_p: 0.95,
            max_tokens: 256,
            stop: vec!["END".into()],
            seed: Some(42),
        };
        let body = backend.build_body(&req, true);
        assert_eq!(body["stream"], true);
        assert_eq!(body["stop"][0], "END");
        assert_eq!(body["seed"], 42);
    }
}
