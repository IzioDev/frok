use quinn::{Endpoint, Incoming};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

use tokio::task::JoinHandle;
use tracing::debug;

use crate::state::EdgeState;
use crate::transport;

pub struct QuicListener {
    cancellation_token: CancellationToken,
    endpoint: Endpoint,
    state: Arc<EdgeState>,
}

impl QuicListener {
    pub fn new(
        endpoint: Endpoint,
        cancellation_token: CancellationToken,
        state: Arc<EdgeState>,
    ) -> Self {
        Self {
            cancellation_token,
            endpoint,
            state,
        }
    }

    pub fn start_listen_task(&self) -> JoinHandle<()> {
        let endpoint = self.endpoint.clone();
        let token = self.cancellation_token.clone();
        let state = self.state.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = token.cancelled() => {
                        endpoint.close(0u32.into(), b"shutdown");
                        break;
                    }
                    incoming = Self::next_accept(&endpoint) => {
                        if let Some(incoming) = incoming {
                            transport::spawn_incoming(incoming, state.clone());
                        }
                    }
                }
            }
        })
    }

    async fn next_accept(endpoint: &Endpoint) -> Option<Incoming> {
        let incoming = endpoint.accept().await;

        if let Some(incoming) = incoming {
            let addr = incoming.remote_address();

            // TODO: optional filtering / ban / rate-limit
            debug!(remote = %addr, "accepted incoming connection");

            return Some(incoming);
        }

        incoming
    }
}
