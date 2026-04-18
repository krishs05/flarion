use std::sync::Arc;
use std::time::Instant;

use crate::admin::tracker::RequestTracker;
use crate::config::RouteConfig;
use crate::engine::registry::BackendRegistry;

pub struct AdminState {
    pub registry: Arc<BackendRegistry>,
    pub routes: Vec<RouteConfig>,
    pub bind: String,
    pub started_at: Instant,
    pub tracker: Arc<RequestTracker>,
}

impl AdminState {
    pub fn new(
        registry: Arc<BackendRegistry>,
        routes: Vec<RouteConfig>,
        bind: String,
        history_size: usize,
    ) -> Self {
        Self {
            registry,
            routes,
            bind,
            started_at: Instant::now(),
            tracker: Arc::new(RequestTracker::new(history_size)),
        }
    }

    pub fn uptime_s(&self) -> u64 {
        self.started_at.elapsed().as_secs()
    }
}
