use serde::{Deserialize, Serialize};

// ── Request Types ──

#[derive(Debug, Deserialize, Clone)]
pub struct ChatCompletionRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    #[serde(default)]
    pub stream: bool,
    #[serde(default = "default_temperature")]
    pub temperature: f32,
    #[serde(default = "default_top_p")]
    pub top_p: f32,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    #[serde(default)]
    pub stop: Vec<String>,
    pub seed: Option<u32>,
}

fn default_temperature() -> f32 { 0.7 }
fn default_top_p() -> f32 { 0.9 }
fn default_max_tokens() -> u32 { 2048 }

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

// ── Non-Streaming Response ──

#[derive(Debug, Serialize, Clone)]
pub struct ChatCompletionResponse {
    pub id: String,
    pub object: String,
    pub created: i64,
    pub model: String,
    pub choices: Vec<ChatCompletionChoice>,
    pub usage: Usage,
}

#[derive(Debug, Serialize, Clone)]
pub struct ChatCompletionChoice {
    pub index: u32,
    pub message: ChatMessage,
    pub finish_reason: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

// ── Streaming Response ──

#[derive(Debug, Serialize, Clone)]
pub struct ChatCompletionChunk {
    pub id: String,
    pub object: String,
    pub created: i64,
    pub model: String,
    pub choices: Vec<ChatCompletionChunkChoice>,
}

#[derive(Debug, Serialize, Clone)]
pub struct ChatCompletionChunkChoice {
    pub index: u32,
    pub delta: ChatDelta,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct ChatDelta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
}

// ── Models Endpoint ──

#[derive(Debug, Serialize)]
pub struct ModelsResponse {
    pub object: String,
    pub data: Vec<ModelObject>,
}

#[derive(Debug, Serialize)]
pub struct ModelObject {
    pub id: String,
    pub object: String,
    pub created: i64,
    pub owned_by: String,
}

// ── Health Endpoint ──

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
    pub model_loaded: bool,
    pub model_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deserialize_minimal_request() {
        let json = r#"{"model": "test-model", "messages": [{"role": "user", "content": "hello"}]}"#;
        let req: ChatCompletionRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.model, "test-model");
        assert_eq!(req.messages.len(), 1);
        assert!(!req.stream);
        assert_eq!(req.temperature, 0.7);
        assert_eq!(req.top_p, 0.9);
        assert_eq!(req.max_tokens, 2048);
        assert!(req.stop.is_empty());
        assert!(req.seed.is_none());
    }

    #[test]
    fn test_deserialize_full_request() {
        let json = r#"{"model":"qwen3-8b","messages":[{"role":"system","content":"You are helpful."},{"role":"user","content":"Hi"}],"stream":true,"temperature":0.5,"top_p":0.95,"max_tokens":1024,"stop":["\n\n"],"seed":42}"#;
        let req: ChatCompletionRequest = serde_json::from_str(json).unwrap();
        assert!(req.stream);
        assert_eq!(req.temperature, 0.5);
        assert_eq!(req.max_tokens, 1024);
        assert_eq!(req.seed, Some(42));
        assert_eq!(req.stop, vec!["\n\n"]);
    }

    #[test]
    fn test_serialize_response() {
        let resp = ChatCompletionResponse {
            id: "chatcmpl-123".to_string(),
            object: "chat.completion".to_string(),
            created: 1700000000,
            model: "test".to_string(),
            choices: vec![ChatCompletionChoice {
                index: 0,
                message: ChatMessage { role: "assistant".to_string(), content: "Hello!".to_string() },
                finish_reason: "stop".to_string(),
            }],
            usage: Usage { prompt_tokens: 10, completion_tokens: 5, total_tokens: 15 },
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["object"], "chat.completion");
        assert_eq!(json["choices"][0]["finish_reason"], "stop");
        assert_eq!(json["usage"]["total_tokens"], 15);
    }

    #[test]
    fn test_serialize_chunk_skips_none_fields() {
        let chunk = ChatCompletionChunk {
            id: "chatcmpl-123".to_string(),
            object: "chat.completion.chunk".to_string(),
            created: 1700000000,
            model: "test".to_string(),
            choices: vec![ChatCompletionChunkChoice {
                index: 0,
                delta: ChatDelta { role: None, content: Some("Hi".to_string()) },
                finish_reason: None,
            }],
        };
        let json = serde_json::to_string(&chunk).unwrap();
        assert!(!json.contains("\"role\""));
        assert!(json.contains("\"content\":\"Hi\""));
    }

    #[test]
    fn test_serialize_models_response() {
        let resp = ModelsResponse {
            object: "list".to_string(),
            data: vec![ModelObject {
                id: "qwen3-8b".to_string(),
                object: "model".to_string(),
                created: 1700000000,
                owned_by: "local".to_string(),
            }],
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["object"], "list");
        assert_eq!(json["data"][0]["owned_by"], "local");
    }
}
