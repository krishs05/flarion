use std::time::Duration;

use eventsource_stream::Eventsource;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};

use crate::api::types::{
    ChatCompletionChoice, ChatCompletionChunk, ChatCompletionChunkChoice, ChatCompletionRequest,
    ChatCompletionResponse, ChatDelta, ChatMessage, Usage,
};
use crate::config::ModelConfig;
use crate::engine::backend::{InferenceBackend, ModelInfo};
use crate::error::EngineError;

const ANTHROPIC_DEFAULT_BASE_URL: &str = "https://api.anthropic.com/v1";
const ANTHROPIC_VERSION: &str = "2023-06-01";

pub struct AnthropicBackend {
    id: String,
    upstream_model: String,
    base_url: String,
    api_key: String,
    client: reqwest::Client,
    max_tokens_cap: u32,
}

impl AnthropicBackend {
    pub fn new(cfg: &ModelConfig) -> Result<Self, EngineError> {
        let api_key = cfg.api_key.clone().ok_or_else(|| {
            EngineError::ModelLoadFailed(format!(
                "model '{}': api_key required for anthropic backend",
                cfg.id
            ))
        })?;

        let upstream_model = cfg.upstream_model.clone().unwrap_or_else(|| cfg.id.clone());

        let base_url = cfg
            .base_url
            .clone()
            .unwrap_or_else(|| ANTHROPIC_DEFAULT_BASE_URL.to_string());

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
            base_url,
            api_key,
            client,
            max_tokens_cap,
        })
    }

    fn endpoint(&self) -> String {
        format!("{}/messages", self.base_url.trim_end_matches('/'))
    }

    fn upstream_error(&self, status: reqwest::StatusCode, body: &str) -> EngineError {
        let upstream_message = serde_json::from_str::<serde_json::Value>(body)
            .ok()
            .and_then(|v| {
                v.get("error")
                    .and_then(|e| e.get("message"))
                    .and_then(|m| m.as_str())
                    .map(String::from)
            })
            .unwrap_or_else(|| body.chars().take(200).collect::<String>());

        let full = format!("upstream anthropic {}: {}", status, upstream_message);

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
            EngineError::InferenceFailed(format!("upstream anthropic request error: {err}"))
        }
    }
}

#[derive(Debug, Serialize)]
struct AnthropicRequest {
    model: String,
    messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stop_sequences: Option<Vec<String>>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    stream: bool,
}

#[derive(Debug, Serialize)]
struct AnthropicMessage {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct AnthropicResponse {
    id: String,
    model: String,
    content: Vec<AnthropicContent>,
    stop_reason: Option<String>,
    usage: AnthropicUsage,
}

#[derive(Debug, Deserialize)]
struct AnthropicContent {
    #[serde(rename = "type")]
    content_type: String,
    #[serde(default)]
    text: String,
}

#[derive(Debug, Deserialize)]
struct AnthropicUsage {
    input_tokens: u32,
    output_tokens: u32,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
#[allow(dead_code)]
enum AnthropicStreamEvent {
    #[serde(rename = "message_start")]
    MessageStart { message: serde_json::Value },
    #[serde(rename = "content_block_start")]
    ContentBlockStart {
        index: u32,
        content_block: serde_json::Value,
    },
    #[serde(rename = "content_block_delta")]
    ContentBlockDelta {
        index: u32,
        delta: AnthropicContentDelta,
    },
    #[serde(rename = "content_block_stop")]
    ContentBlockStop { index: u32 },
    #[serde(rename = "message_delta")]
    MessageDelta {
        delta: AnthropicMessageDelta,
        usage: AnthropicUsageDelta,
    },
    #[serde(rename = "message_stop")]
    MessageStop,
    #[serde(rename = "ping")]
    Ping,
    #[serde(rename = "error")]
    Error { error: serde_json::Value },
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct AnthropicContentDelta {
    #[serde(rename = "type")]
    delta_type: String,
    #[serde(default)]
    text: String,
}

#[derive(Debug, Deserialize)]
struct AnthropicMessageDelta {
    stop_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct AnthropicUsageDelta {
    output_tokens: u32,
}

fn to_anthropic_request(
    req: &ChatCompletionRequest,
    upstream_model: &str,
    stream: bool,
) -> AnthropicRequest {
    let mut system: Option<String> = None;
    let mut messages: Vec<AnthropicMessage> = Vec::new();

    for m in &req.messages {
        match m.role.as_str() {
            "system" => {
                system = Some(match system.take() {
                    Some(prev) => format!("{prev}\n\n{}", m.content),
                    None => m.content.clone(),
                });
            }
            other => messages.push(AnthropicMessage {
                role: other.to_string(),
                content: m.content.clone(),
            }),
        }
    }

    AnthropicRequest {
        model: upstream_model.to_string(),
        messages,
        system,
        max_tokens: req.max_tokens,
        temperature: Some(req.temperature),
        top_p: Some(req.top_p),
        stop_sequences: if req.stop.is_empty() {
            None
        } else {
            Some(req.stop.clone())
        },
        stream,
    }
}

fn map_stop_reason(reason: Option<&str>) -> String {
    match reason {
        Some("end_turn") => "stop",
        Some("max_tokens") => "length",
        Some("stop_sequence") => "stop",
        _ => "stop",
    }
    .to_string()
}

fn from_anthropic_response(
    resp: AnthropicResponse,
    local_id: String,
    request_id: String,
    created: i64,
) -> ChatCompletionResponse {
    let content = resp
        .content
        .into_iter()
        .filter(|c| c.content_type == "text")
        .map(|c| c.text)
        .collect::<Vec<_>>()
        .join("");

    let finish_reason = map_stop_reason(resp.stop_reason.as_deref());

    ChatCompletionResponse {
        id: request_id,
        object: "chat.completion".to_string(),
        created,
        model: local_id,
        choices: vec![ChatCompletionChoice {
            index: 0,
            message: ChatMessage {
                role: "assistant".to_string(),
                content,
            },
            finish_reason,
        }],
        usage: Usage {
            prompt_tokens: resp.usage.input_tokens,
            completion_tokens: resp.usage.output_tokens,
            total_tokens: resp.usage.input_tokens + resp.usage.output_tokens,
        },
    }
}

#[async_trait::async_trait]
impl InferenceBackend for AnthropicBackend {
    async fn load(&self) -> Result<(), EngineError> {
        Ok(())
    }

    async fn chat_completion(
        &self,
        request: ChatCompletionRequest,
    ) -> Result<ChatCompletionResponse, EngineError> {
        let body = to_anthropic_request(&request, &self.upstream_model, false);
        let response = self
            .client
            .post(self.endpoint())
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
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

        let parsed: AnthropicResponse = response.json().await.map_err(|e| {
            EngineError::InferenceFailed(format!("failed to parse anthropic response: {e}"))
        })?;

        let request_id = format!("chatcmpl-{}", uuid::Uuid::new_v4());
        let created = chrono::Utc::now().timestamp();

        Ok(from_anthropic_response(
            parsed,
            self.id.clone(),
            request_id,
            created,
        ))
    }

    async fn chat_completion_stream(
        &self,
        request: ChatCompletionRequest,
        tx: tokio::sync::mpsc::Sender<ChatCompletionChunk>,
    ) -> Result<(), EngineError> {
        let body = to_anthropic_request(&request, &self.upstream_model, true);
        let response = self
            .client
            .post(self.endpoint())
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
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

        let request_id = format!("chatcmpl-{}", uuid::Uuid::new_v4());
        let created = chrono::Utc::now().timestamp();
        let mut events = response.bytes_stream().eventsource();

        while let Some(event) = events.next().await {
            let event = match event {
                Ok(e) => e,
                Err(e) => {
                    tracing::warn!(error = %e, "anthropic SSE stream error");
                    break;
                }
            };

            let parsed: AnthropicStreamEvent = match serde_json::from_str(&event.data) {
                Ok(p) => p,
                Err(e) => {
                    // Never log `event.data` — it may contain prompt echoes
                    // or user content. Length is enough for triage.
                    tracing::warn!(
                        error = %e,
                        data_len = event.data.len(),
                        "failed to parse anthropic event"
                    );
                    continue;
                }
            };

            match parsed {
                AnthropicStreamEvent::MessageStart { .. } => {
                    let chunk = ChatCompletionChunk {
                        id: request_id.clone(),
                        object: "chat.completion.chunk".to_string(),
                        created,
                        model: self.id.clone(),
                        choices: vec![ChatCompletionChunkChoice {
                            index: 0,
                            delta: ChatDelta {
                                role: Some("assistant".to_string()),
                                content: None,
                            },
                            finish_reason: None,
                        }],
                    };
                    if tx.send(chunk).await.is_err() {
                        break;
                    }
                }
                AnthropicStreamEvent::ContentBlockDelta { delta, .. } => {
                    if delta.text.is_empty() {
                        continue;
                    }
                    let chunk = ChatCompletionChunk {
                        id: request_id.clone(),
                        object: "chat.completion.chunk".to_string(),
                        created,
                        model: self.id.clone(),
                        choices: vec![ChatCompletionChunkChoice {
                            index: 0,
                            delta: ChatDelta {
                                role: None,
                                content: Some(delta.text),
                            },
                            finish_reason: None,
                        }],
                    };
                    if tx.send(chunk).await.is_err() {
                        break;
                    }
                }
                AnthropicStreamEvent::MessageDelta { delta, .. } => {
                    if let Some(reason) = delta.stop_reason {
                        let chunk = ChatCompletionChunk {
                            id: request_id.clone(),
                            object: "chat.completion.chunk".to_string(),
                            created,
                            model: self.id.clone(),
                            choices: vec![ChatCompletionChunkChoice {
                                index: 0,
                                delta: ChatDelta {
                                    role: None,
                                    content: None,
                                },
                                finish_reason: Some(map_stop_reason(Some(&reason))),
                            }],
                        };
                        if tx.send(chunk).await.is_err() {
                            break;
                        }
                    }
                }
                AnthropicStreamEvent::Error { error } => {
                    tracing::warn!(error = %error, "anthropic stream error event");
                    break;
                }
                AnthropicStreamEvent::ContentBlockStart { .. }
                | AnthropicStreamEvent::ContentBlockStop { .. }
                | AnthropicStreamEvent::MessageStop
                | AnthropicStreamEvent::Ping => {}
            }
        }

        Ok(())
    }

    fn model_info(&self) -> ModelInfo {
        ModelInfo {
            id: self.id.clone(),
            context_size: 0,
            loaded: true,
            provider: "anthropic".to_string(),
        }
    }

    fn provider(&self) -> &'static str {
        "anthropic"
    }

    fn max_tokens_cap(&self) -> u32 {
        self.max_tokens_cap
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_req(messages: Vec<(&str, &str)>) -> ChatCompletionRequest {
        ChatCompletionRequest {
            model: "claude-x".into(),
            messages: messages
                .into_iter()
                .map(|(role, content)| ChatMessage {
                    role: role.into(),
                    content: content.into(),
                })
                .collect(),
            stream: false,
            temperature: 0.7,
            top_p: 0.9,
            max_tokens: 100,
            stop: vec![],
            seed: None,
        }
    }

    #[test]
    fn test_to_anthropic_extracts_system() {
        let req = make_req(vec![("system", "You are helpful."), ("user", "Hi")]);
        let translated = to_anthropic_request(&req, "claude-3-opus", false);
        assert_eq!(translated.system.as_deref(), Some("You are helpful."));
        assert_eq!(translated.messages.len(), 1);
        assert_eq!(translated.messages[0].role, "user");
        assert_eq!(translated.messages[0].content, "Hi");
    }

    #[test]
    fn test_to_anthropic_combines_multiple_systems() {
        let req = make_req(vec![
            ("system", "First."),
            ("user", "Hi"),
            ("system", "Second."),
        ]);
        let translated = to_anthropic_request(&req, "x", false);
        assert_eq!(translated.system.as_deref(), Some("First.\n\nSecond."));
    }

    #[test]
    fn test_to_anthropic_no_system_means_none() {
        let req = make_req(vec![("user", "Hi"), ("assistant", "Hey")]);
        let translated = to_anthropic_request(&req, "x", false);
        assert!(translated.system.is_none());
        assert_eq!(translated.messages.len(), 2);
    }

    #[test]
    fn test_to_anthropic_maps_stop_sequences() {
        let mut req = make_req(vec![("user", "Hi")]);
        req.stop = vec!["END".to_string(), "STOP".to_string()];
        let translated = to_anthropic_request(&req, "x", false);
        assert_eq!(
            translated.stop_sequences,
            Some(vec!["END".to_string(), "STOP".to_string()])
        );
    }

    #[test]
    fn test_to_anthropic_empty_stop_omitted() {
        let req = make_req(vec![("user", "Hi")]);
        let translated = to_anthropic_request(&req, "x", false);
        assert!(translated.stop_sequences.is_none());
    }

    #[test]
    fn test_to_anthropic_passes_max_tokens() {
        let mut req = make_req(vec![("user", "Hi")]);
        req.max_tokens = 8192;
        let translated = to_anthropic_request(&req, "x", false);
        assert_eq!(translated.max_tokens, 8192);
    }

    #[test]
    fn test_to_anthropic_serializes_correctly() {
        let req = make_req(vec![("system", "S"), ("user", "U")]);
        let translated = to_anthropic_request(&req, "claude", false);
        let json = serde_json::to_value(&translated).unwrap();
        assert_eq!(json["model"], "claude");
        assert_eq!(json["system"], "S");
        assert_eq!(json["messages"][0]["role"], "user");
        assert!(json.get("stream").is_none());
    }

    #[test]
    fn test_from_anthropic_concatenates_text_blocks() {
        let resp = AnthropicResponse {
            id: "msg_1".into(),
            model: "claude".into(),
            content: vec![
                AnthropicContent {
                    content_type: "text".into(),
                    text: "Hello, ".into(),
                },
                AnthropicContent {
                    content_type: "text".into(),
                    text: "world!".into(),
                },
            ],
            stop_reason: Some("end_turn".into()),
            usage: AnthropicUsage {
                input_tokens: 5,
                output_tokens: 3,
            },
        };
        let result =
            from_anthropic_response(resp, "claude-local".into(), "req-id".into(), 1700000000);
        assert_eq!(result.choices[0].message.content, "Hello, world!");
        assert_eq!(result.choices[0].finish_reason, "stop");
        assert_eq!(result.usage.prompt_tokens, 5);
        assert_eq!(result.usage.completion_tokens, 3);
        assert_eq!(result.usage.total_tokens, 8);
        assert_eq!(result.model, "claude-local");
        assert_eq!(result.id, "req-id");
    }

    #[test]
    fn test_from_anthropic_filters_non_text_blocks() {
        let resp = AnthropicResponse {
            id: "x".into(),
            model: "x".into(),
            content: vec![
                AnthropicContent {
                    content_type: "text".into(),
                    text: "keep".into(),
                },
                AnthropicContent {
                    content_type: "tool_use".into(),
                    text: "drop".into(),
                },
            ],
            stop_reason: None,
            usage: AnthropicUsage {
                input_tokens: 0,
                output_tokens: 0,
            },
        };
        let result = from_anthropic_response(resp, "x".into(), "x".into(), 0);
        assert_eq!(result.choices[0].message.content, "keep");
    }

    #[test]
    fn test_map_stop_reason() {
        assert_eq!(map_stop_reason(Some("end_turn")), "stop");
        assert_eq!(map_stop_reason(Some("max_tokens")), "length");
        assert_eq!(map_stop_reason(Some("stop_sequence")), "stop");
        assert_eq!(map_stop_reason(None), "stop");
        assert_eq!(map_stop_reason(Some("unknown")), "stop");
    }

    #[test]
    fn test_endpoint_handles_trailing_slash() {
        let cfg = ModelConfig {
            id: "claude-x".into(),
            backend: crate::config::BackendType::Anthropic,
            path: None,
            context_size: 4096,
            gpu_layers: 99,
            threads: None,
            batch_size: None,
            seed: None,
            api_key: Some("sk-ant".into()),
            base_url: Some("https://api.anthropic.com/v1/".into()),
            upstream_model: None,
            timeout_secs: None,
            max_tokens_cap: None,
            lazy: false,
            vram_mb: None,
        };
        let backend = AnthropicBackend::new(&cfg).unwrap();
        assert_eq!(backend.endpoint(), "https://api.anthropic.com/v1/messages");
    }

    #[test]
    fn test_provider_field() {
        let cfg = ModelConfig {
            id: "claude-x".into(),
            backend: crate::config::BackendType::Anthropic,
            path: None,
            context_size: 4096,
            gpu_layers: 99,
            threads: None,
            batch_size: None,
            seed: None,
            api_key: Some("sk-ant".into()),
            base_url: None,
            upstream_model: None,
            timeout_secs: None,
            max_tokens_cap: None,
            lazy: false,
            vram_mb: None,
        };
        let backend = AnthropicBackend::new(&cfg).unwrap();
        assert_eq!(backend.provider(), "anthropic");
        assert_eq!(backend.model_info().provider, "anthropic");
    }

    #[test]
    fn test_parse_content_block_delta_event() {
        let raw =
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hi"}}"#;
        let parsed: AnthropicStreamEvent = serde_json::from_str(raw).unwrap();
        match parsed {
            AnthropicStreamEvent::ContentBlockDelta { delta, .. } => {
                assert_eq!(delta.text, "Hi");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_parse_message_delta_event_with_stop_reason() {
        let raw = r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":5}}"#;
        let parsed: AnthropicStreamEvent = serde_json::from_str(raw).unwrap();
        match parsed {
            AnthropicStreamEvent::MessageDelta { delta, .. } => {
                assert_eq!(delta.stop_reason.as_deref(), Some("end_turn"));
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_parse_ping_event() {
        let raw = r#"{"type":"ping"}"#;
        let parsed: AnthropicStreamEvent = serde_json::from_str(raw).unwrap();
        assert!(matches!(parsed, AnthropicStreamEvent::Ping));
    }
}
