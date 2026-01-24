use anyhow::Result;
use async_trait::async_trait;
use frok_common::wire::write_wire_message;
use quinn::SendStream;
use tokio::sync::Mutex;

use frok_protocol::{WireMessage, WireSender};

pub struct StreamSender {
    inner: Mutex<SendStream>,
}

impl StreamSender {
    pub fn new(send: SendStream) -> Self {
        Self {
            inner: Mutex::new(send),
        }
    }

    pub async fn finish(&self) {
        let mut send = self.inner.lock().await;
        let _ = send.finish();
    }
}

#[async_trait]
impl WireSender for StreamSender {
    type Error = anyhow::Error;

    async fn send_wire(&self, message: WireMessage) -> Result<(), Self::Error> {
        let mut send = self.inner.lock().await;
        write_wire_message(&mut *send, &message).await?;
        Ok(())
    }
}
