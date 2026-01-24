use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use http::Method;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProxyMode {
    Http1,
    Http2,
}

#[derive(Clone, Debug)]
pub struct ServiceConfig {
    pub bind_addr: Option<SocketAddr>,
    pub response_bytes: usize,
}

impl Default for ServiceConfig {
    fn default() -> Self {
        Self {
            bind_addr: None,
            response_bytes: 1024,
        }
    }
}

#[derive(Clone, Debug)]
pub struct RouteConfig {
    pub name: String,
    pub mode: ProxyMode,
    pub service: ServiceConfig,
}

#[derive(Clone, Debug)]
pub struct AgentConfig {
    pub label: String,
    pub route: RouteConfig,
    pub bin_path: Option<PathBuf>,
    pub extra_args: Vec<String>,
    pub env: Vec<(String, String)>,
}

#[derive(Clone, Debug)]
pub struct EdgeConfig {
    pub quic_addr: Option<SocketAddr>,
    pub http_addr: Option<SocketAddr>,
    pub host: String,
    pub insecure: bool,
    pub bin_path: Option<PathBuf>,
    pub extra_args: Vec<String>,
    pub env: Vec<(String, String)>,
}

#[derive(Clone, Debug)]
pub struct CeremonyConfig {
    pub edge: EdgeConfig,
    pub agents: Vec<AgentConfig>,
    pub run_dir: Option<PathBuf>,
    pub startup_timeout: Duration,
}

#[derive(Clone, Debug)]
pub struct RequestSpec {
    pub method: Method,
    pub path: String,
    pub body: Vec<u8>,
}

impl Default for RequestSpec {
    fn default() -> Self {
        Self {
            method: Method::GET,
            path: "/".to_string(),
            body: Vec::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct LoadSpec {
    pub total_requests: u64,
    pub concurrency: usize,
    pub request: RequestSpec,
    pub collect_latency: bool,
}

impl Default for LoadSpec {
    fn default() -> Self {
        Self {
            total_requests: 100,
            concurrency: 16,
            request: RequestSpec::default(),
            collect_latency: false,
        }
    }
}

impl Default for EdgeConfig {
    fn default() -> Self {
        Self {
            quic_addr: None,
            http_addr: None,
            host: "127.0.0.1".to_string(),
            insecure: true,
            bin_path: None,
            extra_args: Vec::new(),
            env: Vec::new(),
        }
    }
}

impl Default for CeremonyConfig {
    fn default() -> Self {
        Self::single_http1()
    }
}

impl CeremonyConfig {
    pub fn single(mode: ProxyMode) -> Self {
        let route_name = "app".to_string();
        let agent_label = "agent-1".to_string();
        Self {
            edge: EdgeConfig::default(),
            agents: vec![AgentConfig {
                label: agent_label,
                route: RouteConfig {
                    name: route_name,
                    mode,
                    service: ServiceConfig::default(),
                },
                bin_path: None,
                extra_args: Vec::new(),
                env: Vec::new(),
            }],
            run_dir: None,
            startup_timeout: Duration::from_secs(10),
        }
    }

    pub fn single_http1() -> Self {
        Self::single(ProxyMode::Http1)
    }

    pub fn single_http2() -> Self {
        Self::single(ProxyMode::Http2)
    }

    pub fn heavy_mixed(agent_count: usize) -> Self {
        let mut config = Self {
            edge: EdgeConfig::default(),
            agents: Vec::new(),
            run_dir: None,
            startup_timeout: Duration::from_secs(60),
        };

        for idx in 0..agent_count {
            let mode = if idx % 2 == 0 {
                ProxyMode::Http1
            } else {
                ProxyMode::Http2
            };
            config.agents.push(AgentConfig {
                label: format!("agent-{}", idx + 1),
                route: RouteConfig {
                    name: format!("app-{}", idx + 1),
                    mode,
                    service: ServiceConfig {
                        response_bytes: 8 * 1024,
                        ..Default::default()
                    },
                },
                bin_path: None,
                extra_args: Vec::new(),
                env: Vec::new(),
            });
        }

        config
    }
}
