use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use frok_protocol::IngressMode;

use crate::peer::PeerId;
use crate::state::EdgeState;

#[derive(Clone, Debug)]
pub struct RouteBinding {
    pub hostname: String,
    pub peer_id: PeerId,
    pub mode: IngressMode,
}

#[derive(Clone, Debug)]
pub struct RouteIngressInfo {
    pub mode: IngressMode,
    pub public_port: Option<u16>,
}

#[async_trait]
pub trait IngressRuntime: Send + Sync {
    fn supports(&self, mode: IngressMode) -> bool;

    async fn start(&self, state: Arc<EdgeState>) -> Result<()>;

    async fn register_route(
        &self,
        binding: RouteBinding,
        state: Arc<EdgeState>,
    ) -> Result<RouteIngressInfo>;

    async fn unregister_route(&self, hostname: &str) -> Result<()>;

    async fn shutdown(&self) -> Result<()>;
}

pub struct IngressRegistry {
    http: Arc<dyn IngressRuntime>,
    tcp: Arc<dyn IngressRuntime>,
}

impl IngressRegistry {
    pub fn new(http: Arc<dyn IngressRuntime>, tcp: Arc<dyn IngressRuntime>) -> Arc<Self> {
        Arc::new(Self { http, tcp })
    }

    pub async fn start_all(&self, state: Arc<EdgeState>) -> Result<()> {
        self.http.start(state.clone()).await?;
        self.tcp.start(state).await?;
        Ok(())
    }

    pub async fn register_route(
        &self,
        binding: RouteBinding,
        state: Arc<EdgeState>,
    ) -> Result<RouteIngressInfo> {
        self.runtime_for(binding.mode)?
            .register_route(binding, state)
            .await
    }

    pub async fn unregister_route(&self, mode: IngressMode, hostname: &str) -> Result<()> {
        self.runtime_for(mode)?.unregister_route(hostname).await
    }

    pub async fn shutdown_all(&self) -> Result<()> {
        self.http.shutdown().await?;
        self.tcp.shutdown().await?;
        Ok(())
    }

    fn runtime_for(&self, mode: IngressMode) -> Result<&Arc<dyn IngressRuntime>> {
        let runtime = match mode {
            IngressMode::Http1 | IngressMode::Http2 => &self.http,
            IngressMode::Tcp => &self.tcp,
        };
        if !runtime.supports(mode) {
            anyhow::bail!("ingress runtime does not support {}", mode.label());
        }
        Ok(runtime)
    }
}
