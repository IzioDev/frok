use std::sync::Arc;

use frok_protocol::IngressMode;

use crate::peer::PeerHandle;
use crate::state::EdgeState;

pub struct ResolvedRoute {
    pub peer: Arc<PeerHandle>,
}

pub enum ResolveError {
    MissingRoute,
    ModeMismatch { actual: IngressMode },
    PeerOffline,
}

pub async fn resolve_route_and_peer(
    state: &Arc<EdgeState>,
    hostname: &str,
    expected_mode: IngressMode,
) -> Result<ResolvedRoute, ResolveError> {
    let route = state
        .routes
        .get(hostname)
        .await
        .ok_or(ResolveError::MissingRoute)?;

    if route.mode != expected_mode {
        return Err(ResolveError::ModeMismatch { actual: route.mode });
    }

    let peer = state
        .peers
        .get(route.peer_id)
        .await
        .ok_or(ResolveError::PeerOffline)?;

    Ok(ResolvedRoute { peer })
}
