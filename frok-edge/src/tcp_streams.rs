use crate::peer::PeerId;
use bytes::Bytes;
use dashmap::DashMap;
use frok_common::tcp::TcpStreamEvent;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::mpsc;
use tracing::warn;

#[derive(Clone)]
struct StreamEntry {
    owner: PeerId,
    tx: mpsc::Sender<TcpStreamEvent>,
}

pub struct TcpStreams {
    inner: DashMap<u64, StreamEntry>,
    counter: AtomicU64,
    backpressure: AtomicU64,
}

impl TcpStreams {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: DashMap::new(),
            counter: AtomicU64::new(1),
            backpressure: AtomicU64::new(0),
        })
    }

    pub async fn reserve(&self, owner: PeerId) -> (u64, mpsc::Receiver<TcpStreamEvent>) {
        let id = self.counter.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel(128);
        self.inner.insert(id, StreamEntry { owner, tx });
        (id, rx)
    }

    pub async fn send_data(&self, owner: PeerId, stream_id: u64, chunk: Bytes, end: bool) -> bool {
        let entry = {
            if end {
                self.inner.remove(&stream_id).map(|(_, entry)| entry)
            } else {
                self.inner.get(&stream_id).map(|entry| entry.clone())
            }
        };

        if let Some(entry) = entry {
            if entry.owner != owner {
                return false;
            }
            if entry.tx.capacity() == 0 {
                self.note_backpressure(stream_id, "tcp_stream");
            }
            let event = if end {
                TcpStreamEvent::End
            } else {
                TcpStreamEvent::Data(chunk)
            };
            let _ = entry.tx.send(event).await;
            return true;
        }
        false
    }

    pub async fn close(&self, stream_id: u64) {
        self.inner.remove(&stream_id);
    }

    fn note_backpressure(&self, stream_id: u64, channel: &'static str) {
        let count = self.backpressure.fetch_add(1, Ordering::Relaxed) + 1;
        if count % 256 == 0 {
            warn!(stream_id, channel, count, "tcp stream channel saturated");
        }
    }
}
