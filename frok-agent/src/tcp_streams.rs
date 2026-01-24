use bytes::Bytes;
use dashmap::DashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::mpsc;
use tracing::warn;

#[derive(Debug)]
pub enum TcpStreamEvent {
    Data(Bytes),
    End,
}

pub struct TcpStreams {
    inner: DashMap<u64, mpsc::Sender<TcpStreamEvent>>,
    backpressure: AtomicU64,
}

impl TcpStreams {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: DashMap::new(),
            backpressure: AtomicU64::new(0),
        })
    }

    pub async fn insert(&self, stream_id: u64, tx: mpsc::Sender<TcpStreamEvent>) {
        self.inner.insert(stream_id, tx);
    }

    pub async fn send_data(&self, stream_id: u64, chunk: Bytes, end: bool) -> bool {
        let tx = {
            if end {
                self.inner.remove(&stream_id).map(|(_, tx)| tx)
            } else {
                self.inner.get(&stream_id).map(|entry| entry.clone())
            }
        };

        if let Some(tx) = tx {
            if tx.capacity() == 0 {
                self.note_backpressure(stream_id, "tcp_stream");
            }
            let event = if end {
                TcpStreamEvent::End
            } else {
                TcpStreamEvent::Data(chunk)
            };
            let _ = tx.send(event).await;
            return true;
        }
        false
    }

    pub async fn remove(&self, stream_id: u64) {
        self.inner.remove(&stream_id);
    }

    fn note_backpressure(&self, stream_id: u64, channel: &'static str) {
        let count = self.backpressure.fetch_add(1, Ordering::Relaxed) + 1;
        if count % 256 == 0 {
            warn!(stream_id, channel, count, "tcp stream channel saturated");
        }
    }
}
