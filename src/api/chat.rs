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
use crate::error::ApiError;

/// Hard ceiling for `messages.len()`. The 1 MiB body limit already bounds
/// total size but not per-message overhead (matcher eval, template expansion,
/// tokenizer calls). 100 covers every legitimate chat history; attackers
/// crafting thousands of 10-byte messages hit this first.
const MAX_MESSAGES: usize = 100;
use crate::metrics::{COMPLETION_TOKENS, PROMPT_TOKENS, REQUEST_DURATION_SECONDS, REQUESTS_TOTAL};
use crate::routing::matchers::estimate_prompt_tokens;
use crate::routing::trace::{REQUEST_HEADERS, RouteTrace, with_trace};

/// RAII guard that decrements the in-flight counter when dropped (armed).
/// Used only for the non-streaming path; the guard stays armed so Drop
/// always fires the decrement, even on panic.
struct InFlightGuard {
    admin: Arc<crate::admin::state::AdminState>,
    model_id: String,
    armed: bool,
}

impl InFlightGuard {
    fn new(admin: Arc<crate::admin::state::AdminState>, model_id: String) -> Self {
        admin.tracker.in_flight_inc(&model_id);
        Self { admin, model_id, armed: true }
    }
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        if self.armed {
            self.admin.tracker.in_flight_dec(&self.model_id);
        }
    }
}

/// RAII guard for the streaming path. Moved into `done_stream` so it is
/// consumed (and disarmed) when the terminal SSE future resolves. If the
/// client disconnects before `done_stream` runs, the future is dropped,
/// taking the guard with it; Drop decrements the counter without emitting
/// an event (can't `.await` from Drop).
struct InFlightStreamGuard {
    admin: Option<Arc<crate::admin::state::AdminState>>,
    model_id: String,
    req_id: String,
    started: std::time::Instant,
    backend_id: String,
    armed: bool,
}

impl InFlightStreamGuard {
    /// Called from `done_stream` on clean terminus — disarms, decrements the
    /// counter, emits the admin event classified by `status`.
    /// `status` is one of: "success" | "canceled" | "server_error".
    async fn finalize(mut self, status: &str, err_reason: Option<String>) {
        self.armed = false;
        let Some(admin) = self.admin.clone() else { return };
        admin.tracker.in_flight_dec(&self.model_id);
        let duration_ms = self.started.elapsed().as_millis() as u64;
        let event = match status {
            "success" => crate::admin::types::RequestEvent::Completed {
                id: self.req_id.clone(),
                ts: chrono::Utc::now().to_rfc3339(),
                route: None,
                matched_rule: None,
                backend: self.backend_id.clone(),
                fallback_count: 0,
                status: "ok".into(),
                ttft_ms: None,
                duration_ms,
                prompt_tokens: 0,
                completion_tokens: 0,
            },
            "canceled" => crate::admin::types::RequestEvent::Canceled {
                id: self.req_id.clone(),
                ts: chrono::Utc::now().to_rfc3339(),
                backend: self.backend_id.clone(),
                duration_ms,
            },
            _ => crate::admin::types::RequestEvent::Failed {
                id: self.req_id.clone(),
                ts: chrono::Utc::now().to_rfc3339(),
                backend: self.backend_id.clone(),
                reason: err_reason.unwrap_or_else(|| status.to_string()),
                duration_ms,
            },
        };
        admin.tracker.record(event).await;
    }
}

/// If the SSE stream is dropped mid-flight (client disconnect), `done_stream`
/// is dropped without running, so `finalize` never fires. Drop here balances
/// the counter. No event is emitted because Drop can't `.await`.
impl Drop for InFlightStreamGuard {
    fn drop(&mut self) {
        if self.armed {
            if let Some(admin) = &self.admin {
                admin.tracker.in_flight_dec(&self.model_id);
            }
        }
    }
}

pub async fn chat_completions(
    State(state): State<crate::server::ApiState>,
    headers: HeaderMap,
    Json(mut request): Json<ChatCompletionRequest>,
) -> Result<axum::response::Response, ApiError> {
    let start = Instant::now();
    let registry = state.registry.clone();
    let admin = state.admin.clone();

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
    let model_id = request.model.clone();
    let is_stream = request.stream;

    let req_id = format!("req_{}", uuid::Uuid::new_v4().simple());

    if is_stream {
        // Streaming: response headers are flushed before the first chunk, so
        // the X-Flarion-* trace headers cannot be populated — routing trace
        // is only surfaced via server logs and metrics emitted by
        // RoutedBackend.

        if let Some(admin) = admin.as_ref() {
            admin.tracker.in_flight_inc(&model_id);
        }
        if let Some(admin) = admin.as_ref() {
            admin.tracker.record(crate::admin::types::RequestEvent::Started {
                id: req_id.clone(),
                ts: chrono::Utc::now().to_rfc3339(),
                route: None,
                backend: model_id.clone(),
            }).await;
        }

        let guard = InFlightStreamGuard {
            admin: admin.clone(),
            model_id: model_id.clone(),
            req_id: req_id.clone(),
            started: start,
            backend_id: model_id.clone(),
            armed: true,
        };

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

            // Admin lifecycle event — disarms guard, decrements counter, records event.
            guard.finalize(status, err.clone()).await;

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
        // Non-streaming: full instrumentation with InFlightGuard + lifecycle events.
        let guard = admin.as_ref().map(|a| InFlightGuard::new(a.clone(), model_id.clone()));

        if let Some(admin) = admin.as_ref() {
            admin.tracker.record(crate::admin::types::RequestEvent::Started {
                id: req_id.clone(),
                ts: chrono::Utc::now().to_rfc3339(),
                route: None,
                backend: model_id.clone(),
            }).await;
        }

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
            _ => ("direct".to_string(), model_id.clone()),
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

        // Emit terminal lifecycle event; guard drops at function return and
        // decrements the in-flight counter exactly once.
        if let Some(admin) = admin.as_ref() {
            let duration_ms = start.elapsed().as_millis() as u64;
            let event = match &result {
                Ok(resp) => crate::admin::types::RequestEvent::Completed {
                    id: req_id.clone(),
                    ts: chrono::Utc::now().to_rfc3339(),
                    route: trace.route_id.clone(),
                    matched_rule: trace.rule.clone(),
                    backend: trace.backend_id.clone().unwrap_or_else(|| model_id.clone()),
                    fallback_count: trace.fallback_count,
                    status: "ok".into(),
                    ttft_ms: None,
                    duration_ms,
                    prompt_tokens: resp.usage.prompt_tokens as u64,
                    completion_tokens: resp.usage.completion_tokens as u64,
                },
                Err(e) => crate::admin::types::RequestEvent::Failed {
                    id: req_id.clone(),
                    ts: chrono::Utc::now().to_rfc3339(),
                    backend: trace.backend_id.clone().unwrap_or_else(|| model_id.clone()),
                    reason: e.to_string(),
                    duration_ms,
                },
            };
            admin.tracker.record(event).await;
        }

        // guard drops here, decrementing in-flight counter
        drop(guard);

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
                insert_trace_headers(&mut response_headers, &trace, &model_id);
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

#[cfg(test)]
mod admin_emission_tests {
    use super::*;
    use crate::admin::state::AdminState;
    use crate::api::types::{ChatCompletionRequest, ChatMessage};
    use crate::engine::registry::BackendRegistry;
    use crate::engine::testing::MockBackend;
    use axum::extract::State;

    fn mk_request(model: &str, stream: bool) -> ChatCompletionRequest {
        ChatCompletionRequest {
            model: model.into(),
            messages: vec![ChatMessage { role: "user".into(), content: "hi".into() }],
            stream,
            temperature: 0.7,
            top_p: 0.9,
            max_tokens: 32,
            stop: vec![],
            seed: None,
        }
    }

    #[tokio::test]
    async fn non_streaming_emits_started_and_completed() {
        let mut reg = BackendRegistry::new();
        reg.insert("m".into(), Arc::new(MockBackend::succeeding("m", "hello")));
        let registry = Arc::new(reg);
        let admin = Arc::new(AdminState::new(registry.clone(), Vec::new(), "127.0.0.1:0".into(), 100));

        let state = crate::server::ApiState {
            registry: registry.clone(),
            admin: Some(admin.clone()),
        };
        let _resp = chat_completions(
            State(state),
            HeaderMap::new(),
            Json(mk_request("m", false)),
        ).await.unwrap();

        let events = admin.tracker.tail(10).await;
        assert_eq!(events.len(), 2, "expected Started + Completed, got {} events", events.len());
        assert!(matches!(events[0], crate::admin::types::RequestEvent::Started { .. }));
        assert!(matches!(events[1], crate::admin::types::RequestEvent::Completed { .. }));
        assert_eq!(admin.tracker.in_flight("m"), 0, "counter must be balanced");
    }

    #[tokio::test]
    async fn non_streaming_without_admin_is_silent() {
        let mut reg = BackendRegistry::new();
        reg.insert("m".into(), Arc::new(MockBackend::succeeding("m", "hello")));
        let registry = Arc::new(reg);
        let state = crate::server::ApiState { registry, admin: None };
        let _resp = chat_completions(
            State(state),
            HeaderMap::new(),
            Json(mk_request("m", false)),
        ).await.unwrap();
        // No panic, handler returns Ok — assertion is behavioral absence of effects.
    }

    #[tokio::test]
    async fn streaming_emits_started_and_completed_on_clean_finish() {
        use http_body_util::BodyExt;

        let mut reg = BackendRegistry::new();
        reg.insert("m".into(), Arc::new(MockBackend::streaming_chunks(
            "m",
            vec!["hel".into(), "lo".into()],
        )));
        let registry = Arc::new(reg);
        let admin = Arc::new(AdminState::new(registry.clone(), Vec::new(), "127.0.0.1:0".into(), 100));

        let state = crate::server::ApiState {
            registry: registry.clone(),
            admin: Some(admin.clone()),
        };
        let resp = chat_completions(
            State(state),
            HeaderMap::new(),
            Json(mk_request("m", true)),
        ).await.unwrap();

        // Drain the SSE body so done_stream runs to completion.
        let mut body = resp.into_body();
        while let Some(frame) = body.frame().await {
            let _ = frame;
        }

        // finalize runs inside done_stream, which the drain above awaited.
        let events = admin.tracker.tail(10).await;
        assert!(events.len() >= 2, "expected at least Started + Completed, got: {events:?}");
        assert!(matches!(events.first(), Some(crate::admin::types::RequestEvent::Started { .. })),
            "first event: {:?}", events.first());
        assert!(matches!(events.last(), Some(crate::admin::types::RequestEvent::Completed { .. })),
            "last event: {:?}", events.last());
        assert_eq!(admin.tracker.in_flight("m"), 0, "counter must be balanced");
    }
}
