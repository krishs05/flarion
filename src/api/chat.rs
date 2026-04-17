use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue};
use axum::response::IntoResponse;
use axum::response::sse::{Event, KeepAlive, Sse};
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Instant;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::ReceiverStream;

use crate::api::types::{ChatCompletionChunk, ChatCompletionRequest};
use crate::engine::registry::BackendRegistry;
use crate::error::ApiError;

/// Hard ceiling for `messages.len()`. The 1 MiB body limit already bounds
/// total size but not per-message overhead (matcher eval, template expansion,
/// tokenizer calls). 100 covers every legitimate chat history; attackers
/// crafting thousands of 10-byte messages hit this first.
const MAX_MESSAGES: usize = 100;
use crate::metrics::{COMPLETION_TOKENS, PROMPT_TOKENS, REQUEST_DURATION_SECONDS, REQUESTS_TOTAL};
use crate::routing::matchers::estimate_prompt_tokens;
use crate::routing::trace::{REQUEST_HEADERS, RouteTrace, with_trace};

pub async fn chat_completions(
    State(registry): State<Arc<BackendRegistry>>,
    headers: HeaderMap,
    Json(mut request): Json<ChatCompletionRequest>,
) -> Result<axum::response::Response, ApiError> {
    let start = Instant::now();
    let backend = registry
        .get(&request.model)
        .ok_or_else(|| ApiError::ModelNotFound {
            requested: request.model.clone(),
            available: registry.ids(),
        })?;

    if request.messages.is_empty() {
        return Err(ApiError::BadRequest("messages cannot be empty".to_string()));
    }
    if request.messages.len() > MAX_MESSAGES {
        return Err(ApiError::BadRequest(format!(
            "too many messages: {} (max {MAX_MESSAGES})",
            request.messages.len()
        )));
    }
    // Clamp max_tokens silently — matches OpenAI behavior. Each backend
    // reports its own cap via the trait method; RoutedBackend returns the
    // MIN across its fallback chain.
    let cap = backend.max_tokens_cap();
    if request.max_tokens > cap {
        request.max_tokens = cap;
    }

    let prompt_tokens = estimate_prompt_tokens(&request) as f64;
    let model_id_for_direct = request.model.clone();
    let is_stream = request.stream;

    if is_stream {
        // Streaming: response headers are flushed before the first chunk, so
        // the X-Flarion-* trace headers cannot be populated — routing trace
        // is only surfaced via server logs and metrics emitted by
        // RoutedBackend.
        let (tx, rx) = tokio::sync::mpsc::channel::<ChatCompletionChunk>(256);
        let backend_clone = backend.clone();
        let request_clone = request.clone();
        let header_map = headers_to_map(&headers);

        // Surface a terminal failure (if any) from the background task to the
        // SSE stream. A oneshot is enough: only the first failure matters.
        let (err_tx, err_rx) = tokio::sync::oneshot::channel::<String>();

        tokio::spawn(async move {
            let (result, _trace) = with_trace(async move {
                REQUEST_HEADERS
                    .scope(header_map, async move {
                        backend_clone
                            .chat_completion_stream(request_clone, tx)
                            .await
                    })
                    .await
            })
            .await;
            if let Err(e) = result {
                tracing::error!(error = %e, "streaming inference failed");
                // Opaque client-facing message; detail is in the log above.
                let _ = err_tx.send("upstream streaming error".to_string());
            }
        });

        let saw_finish = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let saw_finish_w = saw_finish.clone();
        let stream = ReceiverStream::new(rx).map(move |chunk| {
            if chunk.choices.iter().any(|c| c.finish_reason.is_some()) {
                saw_finish_w.store(true, std::sync::atomic::Ordering::SeqCst);
            }
            match serde_json::to_string(&chunk) {
                Ok(data) => Ok::<_, Infallible>(Event::default().data(data)),
                Err(e) => {
                    // Serialization failure is a bug in our own types; emit
                    // a visible SSE `error` event instead of a silent drop.
                    tracing::error!(error = %e, "failed to serialize SSE chunk");
                    Ok(Event::default()
                        .event("error")
                        .data(r#"{"error":{"message":"internal serialization error","type":"server_error","code":"internal_error"}}"#))
                }
            }
        });
        let saw_finish_for_counter = saw_finish.clone();
        let done_stream = futures_util::stream::once(async move {
            // Emit a terminal `error` SSE event (if any) before the standard
            // `[DONE]` sentinel, so clients can distinguish clean finish
            // from upstream failure.
            let err = err_rx.await.ok();

            let status = if err.is_some() {
                "server_error"
            } else if saw_finish_for_counter.load(std::sync::atomic::Ordering::SeqCst) {
                "success"
            } else {
                // No finish_reason and no error → client-disconnect cancel.
                "canceled"
            };
            metrics::counter!(
                REQUESTS_TOTAL,
                "route" => "streaming".to_string(),
                "backend" => "streaming".to_string(),
                "status" => status,
            )
            .increment(1);

            let data = match err {
                Some(msg) => {
                    let body = serde_json::json!({
                        "error": { "message": msg, "type": "server_error", "code": "upstream_error" }
                    });
                    return Ok::<_, Infallible>(
                        Event::default().event("error").data(body.to_string()),
                    );
                }
                None => "[DONE]".to_string(),
            };
            Ok::<_, Infallible>(Event::default().data(data))
        });
        let combined = stream.chain(done_stream);

        // route/backend labels aren't known before the first chunk flushes,
        // so use a coarse "streaming" label for the prompt-tokens histogram.
        metrics::histogram!(
            PROMPT_TOKENS,
            "route" => "streaming".to_string(),
            "backend" => "streaming".to_string(),
        )
        .record(prompt_tokens);

        Ok(Sse::new(combined)
            .keep_alive(KeepAlive::default())
            .into_response())
    } else {
        let header_map = headers_to_map(&headers);
        let (result, trace) = with_trace(async move {
            REQUEST_HEADERS
                .scope(
                    header_map,
                    async move { backend.chat_completion(request).await },
                )
                .await
        })
        .await;

        let elapsed = start.elapsed().as_secs_f64();
        let (route_label, backend_label) = match (&trace.route_id, &trace.backend_id) {
            (Some(r), Some(b)) => (r.clone(), b.clone()),
            _ => ("direct".to_string(), model_id_for_direct.clone()),
        };

        metrics::histogram!(
            REQUEST_DURATION_SECONDS,
            "route" => route_label.clone(),
            "backend" => backend_label.clone(),
        )
        .record(elapsed);
        metrics::histogram!(
            PROMPT_TOKENS,
            "route" => route_label.clone(),
            "backend" => backend_label.clone(),
        )
        .record(prompt_tokens);

        match result {
            Ok(response) => {
                metrics::histogram!(
                    COMPLETION_TOKENS,
                    "route" => route_label.clone(),
                    "backend" => backend_label.clone(),
                )
                .record(response.usage.completion_tokens as f64);
                metrics::counter!(
                    REQUESTS_TOTAL,
                    "route" => route_label.clone(),
                    "backend" => backend_label.clone(),
                    "status" => "success",
                )
                .increment(1);

                let mut response_headers = HeaderMap::new();
                insert_trace_headers(&mut response_headers, &trace, &model_id_for_direct);
                Ok((response_headers, Json(response)).into_response())
            }
            Err(err) => {
                metrics::counter!(
                    REQUESTS_TOTAL,
                    "route" => route_label,
                    "backend" => backend_label,
                    "status" => "server_error",
                )
                .increment(1);
                Err(ApiError::from(err))
            }
        }
    }
}

fn headers_to_map(headers: &HeaderMap) -> std::collections::HashMap<String, String> {
    headers
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|v| (name.as_str().to_string(), v.to_string()))
        })
        .collect()
}

fn insert_trace_headers(out: &mut HeaderMap, trace: &RouteTrace, direct_id: &str) {
    if let Some(ref r) = trace.route_id {
        if let Ok(v) = HeaderValue::from_str(r) {
            out.insert("x-flarion-route", v);
        }
    } else {
        out.insert("x-flarion-route", HeaderValue::from_static("direct"));
    }
    if let Some(ref rule) = trace.rule
        && let Ok(v) = HeaderValue::from_str(rule)
    {
        out.insert("x-flarion-rule", v);
    }
    let backend = trace.backend_id.as_deref().unwrap_or(direct_id);
    if let Ok(v) = HeaderValue::from_str(backend) {
        out.insert("x-flarion-backend", v);
    }
    // Fallback count is only meaningful for routed requests.
    if trace.route_id.is_some() {
        out.insert(
            "x-flarion-fallback-count",
            HeaderValue::from(trace.fallback_count),
        );
    }
}
