pub struct RequestTracker {
    _history_size: usize,
}

impl RequestTracker {
    pub fn new(history_size: usize) -> Self {
        Self { _history_size: history_size }
    }
}
