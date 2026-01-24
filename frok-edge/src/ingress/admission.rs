use std::sync::Arc;

use anyhow::Context;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

#[derive(Clone, Debug)]
pub struct AdmissionPolicy {
    pub max_connections: usize,
    pub max_inflight_units: usize,
}

#[derive(Clone)]
pub struct AdmissionController {
    connections: Arc<Semaphore>,
    inflight: Arc<Semaphore>,
}

impl AdmissionController {
    pub fn new(policy: AdmissionPolicy) -> Self {
        Self {
            connections: Arc::new(Semaphore::new(policy.max_connections)),
            inflight: Arc::new(Semaphore::new(policy.max_inflight_units)),
        }
    }

    pub async fn acquire_connection(&self) -> anyhow::Result<OwnedSemaphorePermit> {
        self.connections
            .clone()
            .acquire_owned()
            .await
            .context("admission connection semaphore closed")
    }

    pub async fn acquire_inflight(&self) -> anyhow::Result<OwnedSemaphorePermit> {
        self.inflight
            .clone()
            .acquire_owned()
            .await
            .context("admission inflight semaphore closed")
    }
}
