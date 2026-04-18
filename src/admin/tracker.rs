use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use dashmap::DashMap;
use tokio::sync::{RwLock, broadcast};

use crate::admin::types::RequestEvent;

const BROADCAST_CAPACITY: usize = 256;

pub struct RequestTracker {
    history: RwLock<VecDeque<RequestEvent>>,
    capacity: usize,
    tx: broadcast::Sender<RequestEvent>,
    in_flight: DashMap<String, Arc<AtomicUsize>>,
}

impl RequestTracker {
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(BROADCAST_CAPACITY);
        Self {
            history: RwLock::new(VecDeque::with_capacity(capacity)),
            capacity,
            tx,
            in_flight: DashMap::new(),
        }
    }

    /// Push an event. Fan out to broadcast subscribers (send errors ignored —
    /// a dropped subscriber just means no one is currently listening), then
    /// append to the ring buffer, evicting the oldest entry if at capacity.
    pub async fn record(&self, event: RequestEvent) {
        let _ = self.tx.send(event.clone());
        let mut guard = self.history.write().await;
        if guard.len() == self.capacity {
            guard.pop_front();
        }
        guard.push_back(event);
    }

    /// Return the `n` most recent events in chronological order (oldest first).
    pub async fn tail(&self, n: usize) -> Vec<RequestEvent> {
        let guard = self.history.read().await;
        let skip = guard.len().saturating_sub(n);
        guard.iter().skip(skip).cloned().collect()
    }

    /// Return every event currently in the ring buffer, oldest first.
    pub async fn snapshot_all(&self) -> Vec<RequestEvent> {
        self.history.read().await.iter().cloned().collect()
    }

    /// Subscribe to live events. Each subscriber gets its own receiver;
    /// slow consumers lag and receive RecvError::Lagged(n) on the next recv.
    pub fn subscribe(&self) -> broadcast::Receiver<RequestEvent> {
        self.tx.subscribe()
    }

    fn counter(&self, model_id: &str) -> Arc<AtomicUsize> {
        self.in_flight
            .entry(model_id.to_string())
            .or_insert_with(|| Arc::new(AtomicUsize::new(0)))
            .clone()
    }

    pub fn in_flight_inc(&self, model_id: &str) {
        self.counter(model_id).fetch_add(1, Ordering::SeqCst);
    }

    pub fn in_flight_dec(&self, model_id: &str) {
        self.counter(model_id).fetch_sub(1, Ordering::SeqCst);
    }

    pub fn in_flight(&self, model_id: &str) -> u64 {
        self.in_flight
            .get(model_id)
            .map(|c| c.load(Ordering::SeqCst) as u64)
            .unwrap_or(0)
    }

    pub fn in_flight_total(&self) -> u64 {
        self.in_flight
            .iter()
            .map(|e| e.value().load(Ordering::SeqCst) as u64)
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::types::RequestEvent;

    fn started(id: &str) -> RequestEvent {
        RequestEvent::Started {
            id: id.into(),
            ts: "2026-04-18T00:00:00Z".into(),
            route: None,
            backend: "m".into(),
        }
    }

    #[tokio::test]
    async fn tail_returns_most_recent_events() {
        let t = RequestTracker::new(100);
        t.record(started("a")).await;
        t.record(started("b")).await;
        t.record(started("c")).await;
        let out = t.tail(2).await;
        assert_eq!(out.len(), 2);
        match &out[0] { RequestEvent::Started { id, .. } => assert_eq!(id, "b"), _ => panic!() }
        match &out[1] { RequestEvent::Started { id, .. } => assert_eq!(id, "c"), _ => panic!() }
    }

    #[tokio::test]
    async fn ring_buffer_evicts_oldest_at_capacity() {
        let t = RequestTracker::new(2);
        t.record(started("a")).await;
        t.record(started("b")).await;
        t.record(started("c")).await;
        let out = t.tail(10).await;
        assert_eq!(out.len(), 2);
        match &out[0] { RequestEvent::Started { id, .. } => assert_eq!(id, "b"), _ => panic!() }
        match &out[1] { RequestEvent::Started { id, .. } => assert_eq!(id, "c"), _ => panic!() }
    }

    #[tokio::test]
    async fn broadcast_delivers_events_to_subscribers() {
        let t = RequestTracker::new(10);
        let mut rx = t.subscribe();
        t.record(started("x")).await;
        let got = tokio::time::timeout(std::time::Duration::from_millis(100), rx.recv())
            .await.expect("timed out").expect("recv");
        match got { RequestEvent::Started { id, .. } => assert_eq!(id, "x"), _ => panic!() }
    }

    #[test]
    fn in_flight_increments_and_decrements() {
        let t = RequestTracker::new(10);
        t.in_flight_inc("m");
        t.in_flight_inc("m");
        assert_eq!(t.in_flight("m"), 2);
        t.in_flight_dec("m");
        assert_eq!(t.in_flight("m"), 1);
        assert_eq!(t.in_flight_total(), 1);
    }

    #[test]
    fn in_flight_unknown_model_returns_zero() {
        let t = RequestTracker::new(10);
        assert_eq!(t.in_flight("never-seen"), 0);
    }

    #[tokio::test]
    async fn snapshot_all_returns_full_history() {
        let t = RequestTracker::new(5);
        t.record(started("a")).await;
        t.record(started("b")).await;
        let snap = t.snapshot_all().await;
        assert_eq!(snap.len(), 2);
    }
}
