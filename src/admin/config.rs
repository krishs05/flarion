use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use serde_json::Value;

use crate::admin::state::AdminState;

pub async fn get_config(State(state): State<Arc<AdminState>>) -> Json<Value> {
    let mut v = serde_json::to_value(&*state.config).unwrap_or(serde_json::json!({}));
    redact(&mut v);
    Json(v)
}

/// Walk the JSON value and replace any `api_key`, `api_keys`, or `hf_token_env`
/// field values with `"***"`. Preserves `${VAR}` references verbatim because
/// those are not the secret itself, just a pointer to where it lives.
fn redact(v: &mut Value) {
    match v {
        Value::Object(map) => {
            for (key, val) in map.iter_mut() {
                if is_secret_key(key) {
                    redact_secret(val);
                } else {
                    redact(val);
                }
            }
        }
        Value::Array(arr) => {
            for el in arr.iter_mut() {
                redact(el);
            }
        }
        _ => {}
    }
}

fn redact_secret(v: &mut Value) {
    match v {
        Value::String(s) => {
            if !(s.starts_with("${") && s.ends_with('}')) {
                *s = "***".into();
            }
        }
        Value::Array(arr) => {
            for el in arr.iter_mut() {
                if let Value::String(s) = el {
                    if !(s.starts_with("${") && s.ends_with('}')) {
                        *s = "***".into();
                    }
                }
            }
        }
        _ => {}
    }
}

fn is_secret_key(k: &str) -> bool {
    matches!(k, "api_key" | "api_keys" | "hf_token_env")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn redacts_bare_api_key_string() {
        let mut v = json!({ "api_key": "secret" });
        redact(&mut v);
        assert_eq!(v, json!({ "api_key": "***" }));
    }

    #[test]
    fn preserves_env_var_reference() {
        let mut v = json!({ "api_key": "${MY_KEY}" });
        redact(&mut v);
        assert_eq!(v, json!({ "api_key": "${MY_KEY}" }));
    }

    #[test]
    fn redacts_inside_api_keys_array() {
        let mut v = json!({ "api_keys": ["secret1", "${REF}", "secret2"] });
        redact(&mut v);
        assert_eq!(v, json!({ "api_keys": ["***", "${REF}", "***"] }));
    }

    #[test]
    fn recurses_into_nested_objects() {
        let mut v = json!({
            "server": { "api_keys": ["secret"] },
            "models": [{ "id": "m", "api_key": "abc" }]
        });
        redact(&mut v);
        assert_eq!(v, json!({
            "server": { "api_keys": ["***"] },
            "models": [{ "id": "m", "api_key": "***" }]
        }));
    }

    #[test]
    fn leaves_non_secret_fields_alone() {
        let mut v = json!({ "host": "127.0.0.1", "port": 8080 });
        redact(&mut v);
        assert_eq!(v, json!({ "host": "127.0.0.1", "port": 8080 }));
    }
}
