use anyhow::{Result, anyhow};
use async_trait::async_trait;
use bytes::Bytes;
use frok_protocol::{WireMessage, WireSender, encode_frame};
use quinn::{Connection, RecvStream, SendStream};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::mpsc;
use tracing::warn;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PeerId(u64);

impl fmt::Display for PeerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016x}", self.0)
    }
}

impl PeerId {
    pub fn as_u64(self) -> u64 {
        self.0
    }
}

pub struct PeerHandle {
    pub id: PeerId,
    pub remote: SocketAddr,
    conn: Connection,
    send: mpsc::Sender<Bytes>,
    backpressure: AtomicU64,
}

impl PeerHandle {
    pub fn new(id: PeerId, remote: SocketAddr, conn: Connection, send: SendStream) -> Self {
        let (tx, mut rx) = mpsc::channel::<Bytes>(256);
        let peer_id = id;
        tokio::spawn(async move {
            let mut send = send;
            while let Some(payload) = rx.recv().await {
                if let Err(err) = send.write_all(&payload).await {
                    warn!(peer_id = %peer_id, error = %err, "peer send failed");
                    break;
                }
            }
            let _ = send.finish();
        });
        Self {
            id,
            remote,
            conn,
            send: tx,
            backpressure: AtomicU64::new(0),
        }
    }

    async fn send_bytes(&self, payload: Bytes) -> Result<()> {
        if self.send.capacity() == 0 {
            self.note_backpressure();
        }
        self.send
            .send(payload)
            .await
            .map_err(|_| anyhow!("peer send queue closed"))?;
        Ok(())
    }

    pub async fn open_bi(&self) -> Result<(SendStream, RecvStream)> {
        Ok(self.conn.open_bi().await?)
    }

    fn note_backpressure(&self) {
        let count = self.backpressure.fetch_add(1, Ordering::Relaxed) + 1;
        if count % 256 == 0 {
            warn!(peer_id = %self.id, count, "peer send queue saturated");
        }
    }
}

#[async_trait]
impl WireSender for PeerHandle {
    type Error = anyhow::Error;

    async fn send_wire(&self, message: WireMessage) -> Result<(), Self::Error> {
        let frame = encode_frame(&message)?;
        self.send_bytes(Bytes::from(frame)).await?;
        Ok(())
    }
}

// todo: evaluate other possibilities for a unique id
pub fn make_peer_id(seed: SocketAddr, counter: u64) -> PeerId {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    seed.hash(&mut hasher);
    PeerId(hasher.finish() ^ counter)
}
