use std::sync::{Arc, Mutex};

/// State recorded during routing — set by RoutedBackend, read by the HTTP handler
/// to populate X-Flarion-* response headers.
#[derive(Debug, Clone, Default)]
pub struct RouteTrace {
    pub route_id: Option<String>,
    pub rule: Option<String>,
    pub backend_id: Option<String>,
    pub fallback_count: u32,
}

impl RouteTrace {
    #[allow(dead_code)]
    pub fn direct(backend_id: impl Into<String>) -> Self {
        Self {
            route_id: Some("direct".into()),
            rule: None,
            backend_id: Some(backend_id.into()),
            fallback_count: 0,
        }
    }
}

tokio::task_local! {
    static ROUTE_TRACE: Arc<Mutex<RouteTrace>>;
    /// HTTP request headers installed by the chat handler. RoutedBackend reads
    /// this to evaluate `header_equals` matchers. Leaf backends ignore it.
    pub static REQUEST_HEADERS: std::collections::HashMap<String, String>;
}

/// Run `f` with a fresh RouteTrace installed as a task-local. Returns the
/// future's output and the recorded trace.
pub async fn with_trace<F, T>(f: F) -> (T, RouteTrace)
where
    F: std::future::Future<Output = T>,
{
    let slot = Arc::new(Mutex::new(RouteTrace::default()));
    let slot_ret = slot.clone();
    let output = ROUTE_TRACE.scope(slot, f).await;
    let trace = slot_ret.lock().unwrap().clone();
    (output, trace)
}

/// Access the current task-local trace slot. Returns None if not inside
/// `with_trace` (e.g. when a backend is invoked outside the HTTP handler).
pub fn current() -> Option<Arc<Mutex<RouteTrace>>> {
    ROUTE_TRACE.try_with(|t| t.clone()).ok()
}

/// Update the current trace. No-op if no trace is installed.
pub fn update<F: FnOnce(&mut RouteTrace)>(f: F) {
    if let Some(slot) = current() {
        let mut guard = slot.lock().unwrap();
        f(&mut guard);
    }
}

/// Install the given trace slot into the current task-local scope for `f`.
/// Used when forwarding the parent trace into a `tokio::spawn`ed child task.
pub async fn scope<F, T>(slot: Arc<Mutex<RouteTrace>>, f: F) -> T
where
    F: std::future::Future<Output = T>,
{
    ROUTE_TRACE.scope(slot, f).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn with_trace_collects_updates() {
        let (_, trace) = with_trace(async {
            update(|t| {
                t.route_id = Some("chat".into());
                t.backend_id = Some("m1".into());
                t.fallback_count = 2;
            });
        })
        .await;
        assert_eq!(trace.route_id.as_deref(), Some("chat"));
        assert_eq!(trace.backend_id.as_deref(), Some("m1"));
        assert_eq!(trace.fallback_count, 2);
    }

    #[tokio::test]
    async fn update_is_noop_outside_scope() {
        // Should not panic or error.
        update(|t| t.route_id = Some("x".into()));
        assert!(current().is_none());
    }

    #[tokio::test]
    async fn nested_task_local_propagates_to_futures() {
        let (_, trace) = with_trace(async {
            async {
                update(|t| t.rule = Some("r1".into()));
            }
            .await;
        })
        .await;
        assert_eq!(trace.rule.as_deref(), Some("r1"));
    }
}
