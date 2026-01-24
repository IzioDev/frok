use quinn::{Connection, SendStream};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::RwLock;

use crate::peer::{PeerHandle, PeerId, make_peer_id};

pub struct PeerRegistry {
    inner: RwLock<HashMap<PeerId, Arc<PeerHandle>>>,
    counter: AtomicU64,
}

impl PeerRegistry {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: RwLock::new(HashMap::new()),
            counter: AtomicU64::new(1),
        })
    }

    pub async fn insert(
        &self,
        remote: SocketAddr,
        conn: Connection,
        send: SendStream,
    ) -> Arc<PeerHandle> {
        let id = self.make_id(remote);
        let entry = Arc::new(PeerHandle::new(id, remote, conn, send));

        let mut guard = self.inner.write().await;
        guard.insert(id, entry.clone());
        entry
    }

    pub async fn remove(&self, id: PeerId) -> Option<Arc<PeerHandle>> {
        let mut guard = self.inner.write().await;
        guard.remove(&id)
    }

    pub async fn get(&self, id: PeerId) -> Option<Arc<PeerHandle>> {
        let guard = self.inner.read().await;
        guard.get(&id).cloned()
    }

    fn make_id(&self, remote: SocketAddr) -> PeerId {
        let counter = self.counter.fetch_add(1, Ordering::Relaxed);
        make_peer_id(remote, counter)
    }
}
