use borsh::{BorshDeserialize, BorshSerialize};
use frok_protocol::{IngressMode, RegisteredIngress};

#[derive(Clone, Debug, BorshSerialize, BorshDeserialize)]
pub struct RouteSpec {
    pub local_addr: String,
    pub mode: IngressMode,
}

pub fn format_route(route: &RouteSpec, ingress: &RegisteredIngress) -> String {
    if ingress.mode == IngressMode::Tcp {
        let port = ingress
            .public_port
            .map(|port| format!(":{port}"))
            .unwrap_or_else(|| ":?".to_string());
        format!("{} (tcp {port})", route.local_addr)
    } else {
        format!("{} ({})", route.local_addr, ingress.mode.label())
    }
}
