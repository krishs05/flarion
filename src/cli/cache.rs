use std::time::{Duration, Instant};

use tokio::sync::RwLock;

use crate::admin::types::{Gpu, Model, Route, Status};
use crate::cli::client::FlarionClient;
use crate::cli::error::ClientError;

const TTL: Duration = Duration::from_secs(1);

struct Cached<T: Clone> {
    value: T,
    fetched_at: Instant,
}

impl<T: Clone> Cached<T> {
    fn is_fresh(&self) -> bool {
        self.fetched_at.elapsed() < TTL
    }
}

/// Thin wrapper over `FlarionClient` adding a 1-second TTL cache on the hot
/// dashboard reads (status/gpus/models/routes). Mutations invalidate the
/// cache; non-dashboard reads bypass.
pub struct CachedClient {
    inner: FlarionClient,
    status: RwLock<Option<Cached<Status>>>,
    gpus: RwLock<Option<Cached<Vec<Gpu>>>>,
    models: RwLock<Option<Cached<Vec<Model>>>>,
    routes: RwLock<Option<Cached<Vec<Route>>>>,
}

impl CachedClient {
    pub fn new(inner: FlarionClient) -> Self {
        Self {
            inner,
            status: RwLock::new(None),
            gpus: RwLock::new(None),
            models: RwLock::new(None),
            routes: RwLock::new(None),
        }
    }

    pub fn inner(&self) -> &FlarionClient {
        &self.inner
    }

    pub async fn status(&self) -> Result<Status, ClientError> {
        if let Some(c) = self.status.read().await.as_ref()
            && c.is_fresh() {
                return Ok(c.value.clone());
            }
        let fresh = self.inner.status().await?;
        *self.status.write().await = Some(Cached { value: fresh.clone(), fetched_at: Instant::now() });
        Ok(fresh)
    }

    pub async fn gpus(&self) -> Result<Vec<Gpu>, ClientError> {
        if let Some(c) = self.gpus.read().await.as_ref()
            && c.is_fresh() {
                return Ok(c.value.clone());
            }
        let fresh = self.inner.gpus().await?;
        *self.gpus.write().await = Some(Cached { value: fresh.clone(), fetched_at: Instant::now() });
        Ok(fresh)
    }

    pub async fn models(&self) -> Result<Vec<Model>, ClientError> {
        if let Some(c) = self.models.read().await.as_ref()
            && c.is_fresh() {
                return Ok(c.value.clone());
            }
        let fresh = self.inner.models().await?;
        *self.models.write().await = Some(Cached { value: fresh.clone(), fetched_at: Instant::now() });
        Ok(fresh)
    }

    pub async fn routes(&self) -> Result<Vec<Route>, ClientError> {
        if let Some(c) = self.routes.read().await.as_ref()
            && c.is_fresh() {
                return Ok(c.value.clone());
            }
        let fresh = self.inner.routes().await?;
        *self.routes.write().await = Some(Cached { value: fresh.clone(), fetched_at: Instant::now() });
        Ok(fresh)
    }

    /// Invalidates `status`, `models`, and `gpus` (models and gpus change with
    /// load/unload; status is a superset). `routes` is left cached because
    /// mutations don't change route configuration.
    pub async fn invalidate_on_mutation(&self) {
        *self.status.write().await = None;
        *self.models.write().await = None;
        *self.gpus.write().await = None;
    }

    pub async fn load_model(&self, id: &str) -> Result<(), ClientError> {
        let r = self.inner.load_model(id).await;
        if r.is_ok() {
            self.invalidate_on_mutation().await;
        }
        r
    }

    pub async fn unload_model(&self, id: &str) -> Result<(), ClientError> {
        let r = self.inner.unload_model(id).await;
        if r.is_ok() {
            self.invalidate_on_mutation().await;
        }
        r
    }

    pub async fn pin_model(&self, id: &str, pinned: bool) -> Result<(), ClientError> {
        let r = self.inner.pin_model(id, pinned).await;
        if r.is_ok() {
            self.invalidate_on_mutation().await;
        }
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::endpoint::Endpoint;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn status_body() -> serde_json::Value {
        serde_json::json!({
            "server": { "version": "x", "git_sha": null, "uptime_s": 0, "bind": "b", "features": [] },
            "gpus": [], "models": [], "in_flight_total": 0,
            "recent": { "requests_last_60s": 0, "errors_last_60s": 0, "ttft_p50_ms": null, "ttft_p95_ms": null }
        })
    }

    #[tokio::test]
    async fn two_status_calls_within_ttl_hit_stub_once() {
        let server = MockServer::start().await;
        let hits = Arc::new(AtomicUsize::new(0));
        let hits_c = hits.clone();
        let body = status_body();
        Mock::given(method("GET")).and(path("/v1/admin/status"))
            .respond_with(move |_req: &wiremock::Request| {
                hits_c.fetch_add(1, Ordering::SeqCst);
                ResponseTemplate::new(200).set_body_json(body.clone())
            })
            .mount(&server).await;

        let inner = FlarionClient::new(Endpoint {
            name: "t".into(), url: server.uri(), api_key: None,
        }).unwrap();
        let c = CachedClient::new(inner);
        c.status().await.unwrap();
        c.status().await.unwrap();
        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn mutation_invalidates_status_cache() {
        let server = MockServer::start().await;
        let status_hits = Arc::new(AtomicUsize::new(0));
        let status_hits_c = status_hits.clone();
        let body = status_body();
        Mock::given(method("GET")).and(path("/v1/admin/status"))
            .respond_with(move |_req: &wiremock::Request| {
                status_hits_c.fetch_add(1, Ordering::SeqCst);
                ResponseTemplate::new(200).set_body_json(body.clone())
            })
            .mount(&server).await;
        Mock::given(method("POST")).and(path("/v1/admin/models/m/load"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"loaded":"m"})))
            .mount(&server).await;

        let inner = FlarionClient::new(Endpoint {
            name: "t".into(), url: server.uri(), api_key: None,
        }).unwrap();
        let c = CachedClient::new(inner);
        c.status().await.unwrap();           // 1 hit
        c.load_model("m").await.unwrap();    // invalidates
        c.status().await.unwrap();           // 2 hits (cache was invalidated)
        assert_eq!(status_hits.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn failed_mutation_does_not_invalidate() {
        let server = MockServer::start().await;
        let status_hits = Arc::new(AtomicUsize::new(0));
        let status_hits_c = status_hits.clone();
        let body = status_body();
        Mock::given(method("GET")).and(path("/v1/admin/status"))
            .respond_with(move |_req: &wiremock::Request| {
                status_hits_c.fetch_add(1, Ordering::SeqCst);
                ResponseTemplate::new(200).set_body_json(body.clone())
            })
            .mount(&server).await;
        Mock::given(method("POST")).and(path("/v1/admin/models/m/unload"))
            .respond_with(ResponseTemplate::new(409).set_body_string("busy"))
            .mount(&server).await;

        let inner = FlarionClient::new(Endpoint {
            name: "t".into(), url: server.uri(), api_key: None,
        }).unwrap();
        let c = CachedClient::new(inner);
        c.status().await.unwrap();                                  // 1 hit
        let _ = c.unload_model("m").await;                          // fails with 409
        c.status().await.unwrap();                                  // still 1 hit (cache intact)
        assert_eq!(status_hits.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn gpus_and_models_are_independently_cached() {
        let server = MockServer::start().await;
        let gpus_hits = Arc::new(AtomicUsize::new(0));
        let gpus_hits_c = gpus_hits.clone();
        let models_hits = Arc::new(AtomicUsize::new(0));
        let models_hits_c = models_hits.clone();
        Mock::given(method("GET")).and(path("/v1/admin/gpus"))
            .respond_with(move |_req: &wiremock::Request| {
                gpus_hits_c.fetch_add(1, Ordering::SeqCst);
                ResponseTemplate::new(200).set_body_json(serde_json::json!([]))
            })
            .mount(&server).await;
        Mock::given(method("GET")).and(path("/v1/admin/models"))
            .respond_with(move |_req: &wiremock::Request| {
                models_hits_c.fetch_add(1, Ordering::SeqCst);
                ResponseTemplate::new(200).set_body_json(serde_json::json!([]))
            })
            .mount(&server).await;

        let inner = FlarionClient::new(Endpoint {
            name: "t".into(), url: server.uri(), api_key: None,
        }).unwrap();
        let c = CachedClient::new(inner);
        c.gpus().await.unwrap();
        c.gpus().await.unwrap();
        c.models().await.unwrap();
        c.models().await.unwrap();
        assert_eq!(gpus_hits.load(Ordering::SeqCst), 1);
        assert_eq!(models_hits.load(Ordering::SeqCst), 1);
    }
}
