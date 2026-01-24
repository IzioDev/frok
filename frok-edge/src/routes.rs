use crate::peer::PeerId;
use frok_common::normalize_host;
use frok_protocol::IngressMode;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Clone, Debug)]
pub struct RouteEntry {
    pub peer_id: PeerId,
    pub subject: String,
    pub local_addr: String,
    pub mode: IngressMode,
    pub ingress_port: Option<u16>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RegisterResult {
    Ok,
    AlreadyTaken,
}

pub struct RouteRegistry {
    inner: RwLock<HashMap<String, RouteEntry>>,
}

impl RouteRegistry {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: RwLock::new(HashMap::new()),
        })
    }

    pub async fn register(
        &self,
        hostname: String,
        subject: String,
        peer_id: PeerId,
        local_addr: String,
        mode: IngressMode,
    ) -> RegisterResult {
        let host = normalize_host(&hostname);
        let mut guard = self.inner.write().await;
        match guard.get(host.as_ref()) {
            Some(existing) if existing.subject != subject => RegisterResult::AlreadyTaken,
            _ => {
                guard.insert(
                    host.into_owned(),
                    RouteEntry {
                        peer_id,
                        subject,
                        local_addr,
                        mode,
                        ingress_port: None,
                    },
                );
                RegisterResult::Ok
            }
        }
    }

    pub async fn remove_peer(&self, peer_id: PeerId) -> Vec<(String, RouteEntry)> {
        let mut guard = self.inner.write().await;
        let mut removed = Vec::new();
        guard.retain(|host, entry| {
            if entry.peer_id == peer_id {
                removed.push((host.clone(), entry.clone()));
                false
            } else {
                true
            }
        });
        removed
    }

    pub async fn unregister(&self, hostname: &str, peer_id: PeerId) -> Option<RouteEntry> {
        let host = normalize_host(hostname);
        let mut guard = self.inner.write().await;
        match guard.get(host.as_ref()) {
            Some(existing) if existing.peer_id == peer_id => guard.remove(host.as_ref()),
            _ => None,
        }
    }

    pub async fn get(&self, hostname: &str) -> Option<RouteEntry> {
        let host = normalize_host(hostname);
        let guard = self.inner.read().await;
        guard.get(host.as_ref()).cloned()
    }

    pub async fn set_ingress_port(&self, hostname: &str, peer_id: PeerId, port: u16) -> bool {
        let host = normalize_host(hostname);
        let mut guard = self.inner.write().await;
        if let Some(entry) = guard.get_mut(host.as_ref()) {
            if entry.peer_id == peer_id {
                entry.ingress_port = Some(port);
                return true;
            }
        }
        false
    }
}
