mod ceremony;
mod config;
mod load;
mod process;
mod service;
mod util;

pub use ceremony::{Ceremony, RouteEndpoint};
pub use config::{
    AgentConfig, CeremonyConfig, EdgeConfig, LoadSpec, ProxyMode, RequestSpec, RouteConfig,
    ServiceConfig,
};
pub use load::{LatencyReport, LoadReport};

pub async fn run_ceremony(config: CeremonyConfig) -> anyhow::Result<Ceremony> {
    Ceremony::start(config).await
}
