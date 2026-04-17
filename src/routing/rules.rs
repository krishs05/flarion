use std::sync::Arc;
use std::time::Duration;

use crate::config::RouteConfig;
use crate::engine::backend::InferenceBackend;
use crate::routing::matchers::{CompiledMatchers, MatchContext};

#[allow(dead_code)]
pub struct CompiledRule {
    pub name: String,
    pub matchers: CompiledMatchers,
    pub targets: Vec<Arc<dyn InferenceBackend>>,
    pub target_ids: Vec<String>,
    pub first_token_timeout: Duration,
}

#[allow(dead_code)]
pub struct CompiledRoute {
    pub id: String,
    pub rules: Vec<CompiledRule>,
}

impl CompiledRoute {
    #[allow(dead_code)]
    pub fn select_rule(&self, ctx: &MatchContext<'_>) -> Option<&CompiledRule> {
        self.rules.iter().find(|rule| rule.matchers.matches(ctx))
    }
}

/// Resolve leaf backend ids via a provided resolver closure.
pub fn resolve_targets(
    target_ids: &[String],
    resolver: impl Fn(&str) -> Option<Arc<dyn InferenceBackend>>,
) -> Result<Vec<Arc<dyn InferenceBackend>>, String> {
    let mut out = Vec::with_capacity(target_ids.len());
    for id in target_ids {
        match resolver(id) {
            Some(b) => out.push(b),
            None => return Err(id.clone()),
        }
    }
    Ok(out)
}

/// Compile a RouteConfig — regex compile + target resolution.
pub fn compile_route(
    route: &RouteConfig,
    route_default_timeout: Duration,
    resolver: impl Fn(&str) -> Option<Arc<dyn InferenceBackend>>,
) -> Result<CompiledRoute, String> {
    let mut rules = Vec::with_capacity(route.rules.len());
    for rule in &route.rules {
        let matchers = CompiledMatchers::compile(&rule.matchers)
            .map_err(|e| format!("regex compile failed for rule '{}': {e}", rule.name))?;
        let targets = resolve_targets(&rule.targets, &resolver)?;
        let timeout = rule
            .first_token_timeout_ms
            .map(Duration::from_millis)
            .unwrap_or(route_default_timeout);
        rules.push(CompiledRule {
            name: rule.name.clone(),
            matchers,
            targets,
            target_ids: rule.targets.clone(),
            first_token_timeout: timeout,
        });
    }
    Ok(CompiledRoute {
        id: route.id.clone(),
        rules,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::types::{ChatCompletionRequest, ChatMessage};
    use crate::config::Matchers;
    use crate::engine::testing::MockBackend;
    use std::collections::HashMap;

    fn rule(name: &str, matchers: Matchers, target_ids: Vec<&str>) -> CompiledRule {
        CompiledRule {
            name: name.into(),
            matchers: CompiledMatchers::compile(&matchers).unwrap(),
            targets: target_ids
                .iter()
                .map(|id| Arc::new(MockBackend::succeeding(id, "x")) as Arc<dyn InferenceBackend>)
                .collect(),
            target_ids: target_ids.iter().map(|s| s.to_string()).collect(),
            first_token_timeout: Duration::from_secs(5),
        }
    }

    fn req(stream: bool) -> ChatCompletionRequest {
        ChatCompletionRequest {
            model: "m".into(),
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

    #[test]
    fn first_match_wins() {
        let route = CompiledRoute {
            id: "chat".into(),
            rules: vec![
                rule(
                    "stream-only",
                    Matchers {
                        stream: Some(true),
                        ..Matchers::default()
                    },
                    vec!["a"],
                ),
                rule("catchall", Matchers::default(), vec!["b"]),
            ],
        };
        let r = req(true);
        let ctx = MatchContext::from_request(&r, HashMap::new());
        let selected = route.select_rule(&ctx).unwrap();
        assert_eq!(selected.name, "stream-only");
    }

    #[test]
    fn falls_through_to_catchall() {
        let route = CompiledRoute {
            id: "chat".into(),
            rules: vec![
                rule(
                    "stream-only",
                    Matchers {
                        stream: Some(true),
                        ..Matchers::default()
                    },
                    vec!["a"],
                ),
                rule("catchall", Matchers::default(), vec!["b"]),
            ],
        };
        let r = req(false);
        let ctx = MatchContext::from_request(&r, HashMap::new());
        let selected = route.select_rule(&ctx).unwrap();
        assert_eq!(selected.name, "catchall");
    }

    #[test]
    fn returns_none_when_no_rule_matches() {
        let route = CompiledRoute {
            id: "chat".into(),
            rules: vec![rule(
                "stream-only",
                Matchers {
                    stream: Some(true),
                    ..Matchers::default()
                },
                vec!["a"],
            )],
        };
        let r = req(false);
        let ctx = MatchContext::from_request(&r, HashMap::new());
        assert!(route.select_rule(&ctx).is_none());
    }
}
