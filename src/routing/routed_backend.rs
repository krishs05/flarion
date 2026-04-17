use std::collections::HashMap;
use std::time::Instant;

use async_trait::async_trait;

use crate::api::types::{ChatCompletionChunk, ChatCompletionRequest, ChatCompletionResponse};
use crate::engine::backend::{InferenceBackend, ModelInfo};
use crate::error::{EngineError, is_retryable};
use crate::metrics::{
    FALLBACKS_TOTAL, FIRST_TOKEN_SECONDS, REQUESTS_TOTAL, ROUTE_EXHAUSTED_TOTAL,
    ROUTE_RULE_MATCHES_TOTAL,
};
use crate::routing::matchers::MatchContext;
use crate::routing::rules::{CompiledRoute, CompiledRule};
use crate::routing::trace::{REQUEST_HEADERS, update};

pub struct RoutedBackend {
    pub route: CompiledRoute,
    pub context_size: u32,
}

impl RoutedBackend {
    pub fn new(route: CompiledRoute, context_size: u32) -> Self {
        Self {
            route,
            context_size,
        }
    }

    fn select_rule<'a>(&'a self, request: &ChatCompletionRequest) -> Option<&'a CompiledRule> {
        let headers = REQUEST_HEADERS.try_with(|h| h.clone()).unwrap_or_default();
        self.select_rule_with_headers(request, headers)
    }

    pub fn select_rule_with_headers<'a>(
        &'a self,
        request: &ChatCompletionRequest,
        headers: HashMap<String, String>,
    ) -> Option<&'a CompiledRule> {
        let ctx = MatchContext::from_request(request, headers);
        self.route.select_rule(&ctx)
    }

    fn reason_label(err: &EngineError) -> &'static str {
        match err {
            EngineError::Timeout => "timeout",
            EngineError::Network(_) => "network",
            EngineError::UpstreamServerError { .. } => "upstream_5xx",
            EngineError::RateLimited { .. } => "rate_limited",
            _ => "other",
        }
    }
}

#[async_trait]
impl InferenceBackend for RoutedBackend {
    async fn load(&self) -> Result<(), EngineError> {
        Ok(())
    }

    async fn chat_completion(
        &self,
        request: ChatCompletionRequest,
    ) -> Result<ChatCompletionResponse, EngineError> {
        let rule = self
            .select_rule(&request)
            .or_else(|| self.route.rules.first())
            .ok_or_else(|| {
                EngineError::InferenceFailed(format!("route '{}' has no rules", self.route.id))
            })?;

        metrics::counter!(
            ROUTE_RULE_MATCHES_TOTAL,
            "route" => self.route.id.clone(),
            "rule" => rule.name.clone(),
        )
        .increment(1);

        update(|t| {
            t.route_id = Some(self.route.id.clone());
            t.rule = Some(rule.name.clone());
        });

        let mut attempts: Vec<(String, Box<EngineError>)> = Vec::new();
        let mut fallback_count: u32 = 0;

        for (idx, backend) in rule.targets.iter().enumerate() {
            let backend_id = rule.target_ids[idx].clone();
            let start = Instant::now();
            let result = tokio::time::timeout(
                rule.first_token_timeout,
                backend.chat_completion(request.clone()),
            )
            .await;

            let outcome = match result {
                Ok(inner) => inner,
                Err(_) => Err(EngineError::Timeout),
            };

            match outcome {
                Ok(response) => {
                    let elapsed = start.elapsed().as_secs_f64();
                    metrics::histogram!(
                        FIRST_TOKEN_SECONDS,
                        "route" => self.route.id.clone(),
                        "backend" => backend_id.clone(),
                    )
                    .record(elapsed);
                    metrics::counter!(
                        REQUESTS_TOTAL,
                        "route" => self.route.id.clone(),
                        "backend" => backend_id.clone(),
                        "status" => "success",
                    )
                    .increment(1);

                    update(|t| {
                        t.backend_id = Some(backend_id);
                        t.fallback_count = fallback_count;
                    });
                    return Ok(response);
                }
                Err(err) if is_retryable(&err) => {
                    let from_backend = backend_id.clone();
                    let to_backend = rule
                        .target_ids
                        .get(idx + 1)
                        .cloned()
                        .unwrap_or_else(|| "none".into());
                    metrics::counter!(
                        FALLBACKS_TOTAL,
                        "route" => self.route.id.clone(),
                        "from_backend" => from_backend,
                        "to_backend" => to_backend,
                        "reason" => Self::reason_label(&err),
                    )
                    .increment(1);
                    tracing::warn!(
                        route = %self.route.id,
                        backend = %backend_id,
                        error = %err,
                        "retryable error, trying next backend"
                    );
                    attempts.push((backend_id, Box::new(err)));
                    fallback_count += 1;
                    continue;
                }
                Err(err) => return Err(err),
            }
        }

        metrics::counter!(
            ROUTE_EXHAUSTED_TOTAL,
            "route" => self.route.id.clone(),
        )
        .increment(1);

        Err(EngineError::AllBackendsFailed {
            route_id: self.route.id.clone(),
            attempts,
        })
    }

    async fn chat_completion_stream(
        &self,
        request: ChatCompletionRequest,
        client_tx: tokio::sync::mpsc::Sender<ChatCompletionChunk>,
    ) -> Result<(), EngineError> {
        let rule = self
            .select_rule(&request)
            .or_else(|| self.route.rules.first())
            .ok_or_else(|| {
                EngineError::InferenceFailed(format!("route '{}' has no rules", self.route.id))
            })?;

        metrics::counter!(
            ROUTE_RULE_MATCHES_TOTAL,
            "route" => self.route.id.clone(),
            "rule" => rule.name.clone(),
        )
        .increment(1);

        update(|t| {
            t.route_id = Some(self.route.id.clone());
            t.rule = Some(rule.name.clone());
        });

        let mut fallback_count: u32 = 0;
        let mut attempts: Vec<(String, Box<EngineError>)> = Vec::new();

        for (idx, backend) in rule.targets.iter().enumerate() {
            let backend_id = rule.target_ids[idx].clone();
            let (internal_tx, mut internal_rx) =
                tokio::sync::mpsc::channel::<ChatCompletionChunk>(256);
            let backend_clone = backend.clone();
            let request_clone = request.clone();
            let route_id = self.route.id.clone();

            let trace_slot = crate::routing::trace::current();
            let start = Instant::now();
            let handle = tokio::spawn(async move {
                let fut = backend_clone.chat_completion_stream(request_clone, internal_tx);
                match trace_slot {
                    Some(slot) => crate::routing::trace::scope(slot, fut).await,
                    None => fut.await,
                }
            });

            let first = tokio::time::timeout(rule.first_token_timeout, internal_rx.recv()).await;

            match first {
                Ok(Some(chunk)) => {
                    let elapsed = start.elapsed().as_secs_f64();
                    metrics::histogram!(
                        FIRST_TOKEN_SECONDS,
                        "route" => route_id.clone(),
                        "backend" => backend_id.clone(),
                    )
                    .record(elapsed);

                    update(|t| {
                        t.backend_id = Some(backend_id.clone());
                        t.fallback_count = fallback_count;
                    });

                    if client_tx.send(chunk).await.is_err() {
                        handle.abort();
                        return Ok(());
                    }

                    while let Some(chunk) = internal_rx.recv().await {
                        if client_tx.send(chunk).await.is_err() {
                            handle.abort();
                            return Ok(());
                        }
                    }

                    let result = handle.await.unwrap_or(Ok(()));

                    metrics::counter!(
                        REQUESTS_TOTAL,
                        "route" => route_id,
                        "backend" => backend_id,
                        "status" => if result.is_ok() { "success" } else { "server_error" },
                    )
                    .increment(1);
                    return result;
                }
                Ok(None) | Err(_) => {
                    handle.abort();
                    let err = match first {
                        Err(_) => EngineError::Timeout,
                        Ok(None) => EngineError::Network("stream closed before first chunk".into()),
                        _ => unreachable!(),
                    };
                    let to_backend = rule
                        .target_ids
                        .get(idx + 1)
                        .cloned()
                        .unwrap_or_else(|| "none".into());
                    let reason = if matches!(err, EngineError::Timeout) {
                        "first_token_timeout"
                    } else {
                        Self::reason_label(&err)
                    };
                    metrics::counter!(
                        FALLBACKS_TOTAL,
                        "route" => self.route.id.clone(),
                        "from_backend" => backend_id.clone(),
                        "to_backend" => to_backend,
                        "reason" => reason,
                    )
                    .increment(1);
                    tracing::warn!(
                        route = %self.route.id,
                        backend = %backend_id,
                        reason = %reason,
                        "streaming target failed before first chunk; trying next"
                    );
                    attempts.push((backend_id, Box::new(err)));
                    fallback_count += 1;
                    continue;
                }
            }
        }

        metrics::counter!(
            ROUTE_EXHAUSTED_TOTAL,
            "route" => self.route.id.clone(),
        )
        .increment(1);

        Err(EngineError::AllBackendsFailed {
            route_id: self.route.id.clone(),
            attempts,
        })
    }

    fn model_info(&self) -> ModelInfo {
        ModelInfo {
            id: self.route.id.clone(),
            context_size: self.context_size,
            loaded: true,
            provider: "route".to_string(),
        }
    }

    fn provider(&self) -> &'static str {
        "route"
    }

    fn max_tokens_cap(&self) -> u32 {
        // Conservative: return the MIN across all rules' targets. Any fallback
        // in any rule must be able to serve the request, so the effective cap
        // is the smallest cap in the entire route.
        self.route
            .rules
            .iter()
            .flat_map(|rule| rule.targets.iter())
            .map(|b| b.max_tokens_cap())
            .min()
            .unwrap_or(8192)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::types::ChatMessage;
    use crate::config::Matchers;
    use crate::engine::testing::MockBackend;
    use crate::routing::matchers::CompiledMatchers;
    use crate::routing::rules::CompiledRule;
    use std::sync::Arc;
    use std::time::Duration;

    fn req(stream: bool) -> ChatCompletionRequest {
        ChatCompletionRequest {
            model: "route".into(),
            messages: vec![ChatMessage {
                role: "user".into(),
                content: "hi".into(),
            }],
            stream,
            temperature: 0.7,
            top_p: 0.9,
            max_tokens: 16,
            stop: Vec::new(),
            seed: None,
        }
    }

    fn route(rules: Vec<CompiledRule>) -> RoutedBackend {
        RoutedBackend::new(
            CompiledRoute {
                id: "chat".into(),
                rules,
            },
            4096,
        )
    }

    fn rule(
        name: &str,
        targets: Vec<Arc<dyn InferenceBackend>>,
        target_ids: Vec<&str>,
        timeout: Duration,
    ) -> CompiledRule {
        CompiledRule {
            name: name.into(),
            matchers: CompiledMatchers::compile(&Matchers::default()).unwrap(),
            targets,
            target_ids: target_ids.iter().map(|s| s.to_string()).collect(),
            first_token_timeout: timeout,
        }
    }

    #[tokio::test]
    async fn first_target_succeeds_no_fallback() {
        let a = Arc::new(MockBackend::succeeding("a", "ok")) as Arc<dyn InferenceBackend>;
        let routed = route(vec![rule(
            "default",
            vec![a],
            vec!["a"],
            Duration::from_secs(5),
        )]);
        let resp = routed.chat_completion(req(false)).await.unwrap();
        assert_eq!(resp.choices[0].message.content, "ok");
    }

    #[tokio::test]
    async fn retryable_error_triggers_fallback() {
        let a = Arc::new(MockBackend::failing(
            "a",
            EngineError::Network("down".into()),
        )) as Arc<dyn InferenceBackend>;
        let b = Arc::new(MockBackend::succeeding("b", "from-b")) as Arc<dyn InferenceBackend>;
        let routed = route(vec![rule(
            "default",
            vec![a, b],
            vec!["a", "b"],
            Duration::from_secs(5),
        )]);
        let resp = routed.chat_completion(req(false)).await.unwrap();
        assert_eq!(resp.choices[0].message.content, "from-b");
    }

    #[tokio::test]
    async fn non_retryable_error_surfaces_immediately() {
        let a = Arc::new(MockBackend::failing(
            "a",
            EngineError::InferenceFailed("bad request".into()),
        )) as Arc<dyn InferenceBackend>;
        let b = Arc::new(MockBackend::succeeding("b", "from-b")) as Arc<dyn InferenceBackend>;
        let routed = route(vec![rule(
            "default",
            vec![a, b],
            vec!["a", "b"],
            Duration::from_secs(5),
        )]);
        let err = routed.chat_completion(req(false)).await.unwrap_err();
        assert!(matches!(err, EngineError::InferenceFailed(_)));
    }

    #[tokio::test]
    async fn all_fail_returns_all_backends_failed() {
        let a = Arc::new(MockBackend::failing("a", EngineError::Network("x".into())))
            as Arc<dyn InferenceBackend>;
        let b =
            Arc::new(MockBackend::failing("b", EngineError::Timeout)) as Arc<dyn InferenceBackend>;
        let routed = route(vec![rule(
            "default",
            vec![a, b],
            vec!["a", "b"],
            Duration::from_secs(5),
        )]);
        let err = routed.chat_completion(req(false)).await.unwrap_err();
        match err {
            EngineError::AllBackendsFailed { route_id, attempts } => {
                assert_eq!(route_id, "chat");
                assert_eq!(attempts.len(), 2);
                assert_eq!(attempts[0].0, "a");
                assert_eq!(attempts[1].0, "b");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[tokio::test]
    async fn first_token_timeout_triggers_streaming_fallback() {
        let slow = Arc::new(MockBackend::timing_out("slow", Duration::from_secs(60)))
            as Arc<dyn InferenceBackend>;
        let fast = Arc::new(MockBackend::streaming_chunks("fast", vec!["hello".into()]))
            as Arc<dyn InferenceBackend>;
        let routed = route(vec![rule(
            "default",
            vec![slow, fast],
            vec!["slow", "fast"],
            Duration::from_millis(100),
        )]);
        let (tx, mut rx) = tokio::sync::mpsc::channel::<ChatCompletionChunk>(16);
        tokio::spawn(async move {
            routed.chat_completion_stream(req(true), tx).await.unwrap();
        });
        let first = rx.recv().await.unwrap();
        assert_eq!(
            first.choices[0].delta.content.as_deref(),
            Some("hello"),
            "expected first chunk from fast backend"
        );
    }

    #[tokio::test]
    async fn mid_stream_failure_does_not_retry() {
        let primary = Arc::new(MockBackend::streaming_then_error(
            "primary",
            vec!["a".into(), "b".into()],
            EngineError::Network("broke".into()),
        )) as Arc<dyn InferenceBackend>;
        let fallback = Arc::new(MockBackend::streaming_chunks(
            "fallback",
            vec!["SHOULD_NOT_APPEAR".into()],
        )) as Arc<dyn InferenceBackend>;
        let routed = route(vec![rule(
            "default",
            vec![primary, fallback],
            vec!["primary", "fallback"],
            Duration::from_secs(5),
        )]);
        let (tx, mut rx) = tokio::sync::mpsc::channel::<ChatCompletionChunk>(16);
        tokio::spawn(async move {
            let _ = routed.chat_completion_stream(req(true), tx).await;
        });
        let mut texts = Vec::new();
        while let Some(chunk) = rx.recv().await {
            if let Some(c) = chunk.choices[0].delta.content.clone() {
                texts.push(c);
            }
        }
        assert!(
            texts.iter().all(|t| t != "SHOULD_NOT_APPEAR"),
            "fallback should not be invoked after first chunk"
        );
    }
}
