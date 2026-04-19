use std::time::Duration;

use reqwest::{Client, StatusCode};

use crate::admin::types::BuildInfo;
use crate::cli::endpoint::Endpoint;
use crate::cli::error::ClientError;

const DEFAULT_TIMEOUT_SECS: u64 = 5;

pub struct FlarionClient {
    http: Client,
    endpoint: Endpoint,
}

impl FlarionClient {
    pub fn new(endpoint: Endpoint) -> Result<Self, ClientError> {
        let http = Client::builder()
            .user_agent(concat!("flarion-cli/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECS))
            .build()
            .map_err(|e| ClientError::Unreachable {
                url: endpoint.url.clone(),
                source: e,
            })?;
        Ok(Self { http, endpoint })
    }

    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, ClientError> {
        let url = format!("{}{}", self.endpoint.url.trim_end_matches('/'), path);
        let mut req = self.http.get(&url);
        if let Some(k) = &self.endpoint.api_key {
            req = req.bearer_auth(k);
        }
        let resp = req.send().await.map_err(|e| {
            if e.is_timeout() {
                ClientError::Timeout
            } else {
                ClientError::Unreachable {
                    url: url.clone(),
                    source: e,
                }
            }
        })?;
        match resp.status() {
            s if s.is_success() => resp
                .json::<T>()
                .await
                .map_err(|e| ClientError::Decode { reason: e.to_string() }),
            StatusCode::UNAUTHORIZED => Err(ClientError::Unauthorized),
            StatusCode::NOT_FOUND => Err(ClientError::NotFound {
                resource: path.into(),
            }),
            StatusCode::CONFLICT => {
                let body = resp.text().await.unwrap_or_default();
                Err(ClientError::Conflict { reason: body })
            }
            status => {
                let body = resp.text().await.unwrap_or_default();
                Err(ClientError::Server {
                    status: status.as_u16(),
                    body,
                })
            }
        }
    }

    pub async fn version(&self) -> Result<BuildInfo, ClientError> {
        self.get("/v1/admin/version").await
    }

    pub async fn health(&self) -> Result<serde_json::Value, ClientError> {
        self.get("/health").await
    }

    pub async fn status(&self) -> Result<crate::admin::types::Status, ClientError> {
        self.get("/v1/admin/status").await
    }

    pub async fn gpus(&self) -> Result<Vec<crate::admin::types::Gpu>, ClientError> {
        self.get("/v1/admin/gpus").await
    }

    pub async fn models(&self) -> Result<Vec<crate::admin::types::Model>, ClientError> {
        self.get("/v1/admin/models").await
    }

    pub async fn routes(&self) -> Result<Vec<crate::admin::types::Route>, ClientError> {
        self.get("/v1/admin/routes").await
    }

    pub async fn requests(
        &self,
        tail: usize,
    ) -> Result<Vec<crate::admin::types::RequestEvent>, ClientError> {
        self.get(&format!("/v1/admin/requests?tail={tail}")).await
    }

    pub async fn stream_requests(
        &self,
    ) -> Result<
        impl futures_util::Stream<Item = Result<crate::admin::types::RequestEvent, ClientError>>,
        ClientError,
    > {
        use eventsource_stream::Eventsource;
        use futures_util::StreamExt;

        let url = format!(
            "{}/v1/admin/requests/stream",
            self.endpoint.url.trim_end_matches('/')
        );
        let mut req = self.http.get(&url);
        if let Some(k) = &self.endpoint.api_key {
            req = req.bearer_auth(k);
        }
        // SSE streams may idle; no timeout here.
        let req = req.timeout(std::time::Duration::from_secs(60 * 60 * 24));
        let resp = req.send().await.map_err(|e| ClientError::Unreachable {
            url: url.clone(),
            source: e,
        })?;
        if !resp.status().is_success() {
            return Err(match resp.status() {
                reqwest::StatusCode::UNAUTHORIZED => ClientError::Unauthorized,
                s => ClientError::Server {
                    status: s.as_u16(),
                    body: String::new(),
                },
            });
        }
        let byte_stream = resp
            .bytes_stream()
            .map(|r| r.map_err(std::io::Error::other));
        let event_stream = byte_stream.eventsource();
        Ok(event_stream.map(|r| match r {
            Ok(ev) => serde_json::from_str::<crate::admin::types::RequestEvent>(&ev.data)
                .map_err(|e| ClientError::Decode {
                    reason: e.to_string(),
                }),
            Err(e) => Err(ClientError::Stream {
                reason: e.to_string(),
            }),
        }))
    }

    pub async fn effective_config(&self) -> Result<serde_json::Value, ClientError> {
        self.get("/v1/admin/config").await
    }

    async fn post_empty(&self, path: &str) -> Result<(), ClientError> {
        let url = format!("{}{}", self.endpoint.url.trim_end_matches('/'), path);
        let mut req = self.http.post(&url);
        if let Some(k) = &self.endpoint.api_key {
            req = req.bearer_auth(k);
        }
        let resp = req.send().await.map_err(|e| {
            if e.is_timeout() {
                ClientError::Timeout
            } else {
                ClientError::Unreachable {
                    url: url.clone(),
                    source: e,
                }
            }
        })?;
        match resp.status() {
            s if s.is_success() => Ok(()),
            StatusCode::UNAUTHORIZED => Err(ClientError::Unauthorized),
            StatusCode::NOT_FOUND => Err(ClientError::NotFound {
                resource: path.into(),
            }),
            StatusCode::CONFLICT => {
                let body = resp.text().await.unwrap_or_default();
                Err(ClientError::Conflict { reason: body })
            }
            status => {
                let body = resp.text().await.unwrap_or_default();
                Err(ClientError::Server {
                    status: status.as_u16(),
                    body,
                })
            }
        }
    }

    pub async fn load_model(&self, id: &str) -> Result<(), ClientError> {
        self.post_empty(&format!("/v1/admin/models/{id}/load")).await
    }

    pub async fn unload_model(&self, id: &str) -> Result<(), ClientError> {
        self.post_empty(&format!("/v1/admin/models/{id}/unload")).await
    }

    pub async fn pin_model(&self, id: &str, pinned: bool) -> Result<(), ClientError> {
        let action = if pinned { "pin" } else { "unpin" };
        self.post_empty(&format!("/v1/admin/models/{id}/{action}")).await
    }
}
