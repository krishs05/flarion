use std::sync::Arc;

use crate::admin::state::AdminState;
use crate::admin::types::Route;

/// Placeholder — real implementation lands in Task 8 after route configs are
/// threaded through AdminState.
pub fn route_snapshot(_state: &Arc<AdminState>) -> Vec<Route> {
    Vec::new()
}
