use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::extract::{Query, State};
use axum::response::Sse;
use axum::response::sse::{Event, KeepAlive};
use futures_util::Stream;
use serde::Deserialize;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;

use crate::admin::state::AdminState;
use crate::admin::types::RequestEvent;

const MAX_TAIL: usize = 1000;

fn default_tail() -> usize { 50 }

#[derive(Debug, Deserialize)]
pub struct TailQuery {
    #[serde(default = "default_tail")]
    pub tail: usize,
}

pub async fn get_requests(
    State(state): State<Arc<AdminState>>,
    Query(q): Query<TailQuery>,
) -> Json<Vec<RequestEvent>> {
    let n = q.tail.min(MAX_TAIL);
    Json(state.tracker.tail(n).await)
}

pub async fn stream_requests(
    State(state): State<Arc<AdminState>>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = state.tracker.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|result| match result {
        Ok(ev) => match Event::default().json_data(&ev) {
            Ok(e) => Some(Ok(e)),
            Err(err) => {
                tracing::error!(error = %err, "failed to encode RequestEvent as SSE");
                None
            }
        },
        Err(BroadcastStreamRecvError::Lagged(n)) => {
            let gap = RequestEvent::Gap { missed: n };
            match Event::default().json_data(&gap) {
                Ok(e) => Some(Ok(e)),
                Err(err) => {
                    tracing::error!(error = %err, "failed to encode Gap event");
                    None
                }
            }
        }
    });
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}
