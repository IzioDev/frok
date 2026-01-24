use std::sync::Arc;

use crate::auth::EdgeAuth;
use crate::ingress::IngressRegistry;
use crate::registry::PeerRegistry;
use crate::routes::RouteRegistry;
use crate::store::EdgeStore;
use crate::tcp_streams::TcpStreams;

pub struct EdgeState {
    pub peers: Arc<PeerRegistry>,
    pub routes: Arc<RouteRegistry>,
    pub tcp_streams: Arc<TcpStreams>,
    pub ingress: Arc<IngressRegistry>,
    pub store: Arc<EdgeStore>,
    pub auth: Arc<EdgeAuth>,
}

impl EdgeState {
    pub fn new(
        peers: Arc<PeerRegistry>,
        routes: Arc<RouteRegistry>,
        tcp_streams: Arc<TcpStreams>,
        ingress: Arc<IngressRegistry>,
        store: Arc<EdgeStore>,
        auth: Arc<EdgeAuth>,
    ) -> Arc<Self> {
        Arc::new(Self {
            peers,
            routes,
            tcp_streams,
            ingress,
            store,
            auth,
        })
    }
}
