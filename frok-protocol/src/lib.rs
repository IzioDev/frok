use async_trait::async_trait;
use borsh::{BorshDeserialize, BorshSerialize};
use bytes::Bytes;
use std::io::{self, Read, Write};

pub const MAX_FRAME_LEN: u32 = 10 * 1024 * 1024;
pub const MAX_BODY_CHUNK: usize = 256 * 1024;

#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub struct ByteBuf(Bytes);

impl ByteBuf {
    pub fn empty() -> Self {
        Self(Bytes::new())
    }

    pub fn from_static(bytes: &'static [u8]) -> Self {
        Self(Bytes::from_static(bytes))
    }

    pub fn into_bytes(self) -> Bytes {
        self.0
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_ref()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl From<Bytes> for ByteBuf {
    fn from(bytes: Bytes) -> Self {
        Self(bytes)
    }
}

impl From<Vec<u8>> for ByteBuf {
    fn from(bytes: Vec<u8>) -> Self {
        Self(Bytes::from(bytes))
    }
}

impl From<ByteBuf> for Bytes {
    fn from(buf: ByteBuf) -> Self {
        buf.0
    }
}

impl BorshSerialize for ByteBuf {
    fn serialize<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        let len = u32::try_from(self.0.len())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "byte buffer too large"))?;
        len.serialize(writer)?;
        writer.write_all(self.0.as_ref())?;
        Ok(())
    }
}

impl BorshDeserialize for ByteBuf {
    fn deserialize_reader<R: Read>(reader: &mut R) -> io::Result<Self> {
        let len = u32::deserialize_reader(reader)?;
        let mut buf = vec![0u8; len as usize];
        reader.read_exact(&mut buf)?;
        Ok(Self(Bytes::from(buf)))
    }
}

#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct Header {
    pub name: ByteBuf,
    pub value: ByteBuf,
}

impl Header {
    pub fn name_str(&self) -> Option<&str> {
        std::str::from_utf8(self.name.as_bytes()).ok()
    }

    pub fn value_str(&self) -> Option<&str> {
        std::str::from_utf8(self.value.as_bytes()).ok()
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, BorshSerialize, BorshDeserialize)]
pub enum IngressMode {
    Http1,
    Http2,
    Tcp,
}

impl IngressMode {
    pub fn label(self) -> &'static str {
        match self {
            IngressMode::Http1 => "http",
            IngressMode::Http2 => "http2",
            IngressMode::Tcp => "tcp",
        }
    }
}

#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct RegisteredIngress {
    pub mode: IngressMode,
    pub public_port: Option<u16>,
}

#[derive(Debug, Clone, Default, BorshSerialize, BorshDeserialize)]
pub struct BuildInfo {
    pub name: String,
    pub version: String,
    pub commit: Option<String>,
}

#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct HttpRequestStart {
    pub request_id: u64,
    pub method: String,
    pub path: String,
    pub headers: Vec<Header>,
}

#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct HttpRequestBody {
    pub request_id: u64,
    pub chunk: ByteBuf,
    pub end: bool,
}

#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct HttpResponseStart {
    pub request_id: u64,
    pub status: u16,
    pub headers: Vec<Header>,
}

#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct HttpResponseBody {
    pub request_id: u64,
    pub chunk: ByteBuf,
    pub end: bool,
}

#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct TcpStreamOpen {
    pub stream_id: u64,
    pub hostname: String,
}

#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct TcpStreamData {
    pub stream_id: u64,
    pub chunk: ByteBuf,
    pub end: bool,
}

#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct OidcParams {
    pub issuer: String,
    pub audience: String,
    pub allowed_algs: Vec<String>,
}

#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub enum AuthMethod {
    Oidc {
        token: String,
        agent_label: String,
    },
    Key {
        public_key: Vec<u8>,
        signature: Vec<u8>,
        agent_label: String,
    },
}

#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub enum ClientMessage {
    Auth {
        method: AuthMethod,
    },
    Hello {
        agent: String,
        build: BuildInfo,
    },
    Register {
        hostname: String,
        local_addr: String,
        mode: IngressMode,
    },
    Unregister {
        hostname: String,
    },
    HttpResponseStart {
        response: HttpResponseStart,
    },
    HttpResponseBody {
        body: HttpResponseBody,
    },
    TcpStreamData {
        data: TcpStreamData,
    },
    Pong {
        nonce: u64,
    },
}

#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub enum ServerMessage {
    AuthChallenge {
        nonce: Vec<u8>,
        oidc: Option<OidcParams>,
    },
    AuthOk {
        subject: String,
        session_id: u64,
    },
    AuthErr {
        reason: String,
    },
    HelloAck {
        session_id: u64,
        build: BuildInfo,
    },
    RegisterOk {
        hostname: String,
        ingress: RegisteredIngress,
    },
    RegisterErr {
        hostname: String,
        reason: String,
    },
    HttpRequestStart {
        request: HttpRequestStart,
    },
    HttpRequestBody {
        body: HttpRequestBody,
    },
    TcpStreamOpen {
        stream: TcpStreamOpen,
    },
    TcpStreamData {
        data: TcpStreamData,
    },
    Ping {
        nonce: u64,
    },
}

#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub enum WireMessage {
    Client(ClientMessage),
    Server(ServerMessage),
}

pub fn encode_frame(message: &WireMessage) -> io::Result<Vec<u8>> {
    let mut buf = Vec::with_capacity(4);
    buf.extend_from_slice(&[0u8; 4]);

    message.serialize(&mut buf)?;

    let payload_len = buf.len() - 4;
    if payload_len > MAX_FRAME_LEN as usize {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame exceeds MAX_FRAME_LEN",
        ));
    }

    buf[..4].copy_from_slice(&(payload_len as u32).to_le_bytes());
    Ok(buf)
}

pub fn decode_frame(payload: &[u8]) -> io::Result<WireMessage> {
    WireMessage::try_from_slice(payload)
}

pub fn read_framed<R: Read>(reader: &mut R, buffer: &mut Vec<u8>) -> io::Result<WireMessage> {
    let mut len_bytes = [0u8; 4];
    reader.read_exact(&mut len_bytes)?;

    let len = u32::from_le_bytes(len_bytes);
    if len > MAX_FRAME_LEN {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame exceeds MAX_FRAME_LEN",
        ));
    }
    let len = len as usize;

    buffer.clear();

    buffer.reserve(len);
    // SAFETY: immediately fill exactly `len` bytes via read_exact before decoding
    unsafe { buffer.set_len(len) };
    // ---

    reader.read_exact(&mut buffer[..])?;

    decode_frame(buffer)
}

pub fn write_framed<W: Write>(writer: &mut W, message: &WireMessage) -> io::Result<()> {
    let frame = encode_frame(message)?;
    writer.write_all(&frame)
}

#[async_trait]
pub trait WireSender {
    type Error: Send + Sync + 'static;

    async fn send_wire(&self, message: WireMessage) -> Result<(), Self::Error>;
}

#[async_trait]
pub trait ClientCommands: WireSender {
    async fn auth(&self, method: AuthMethod) -> Result<(), Self::Error> {
        self.send_wire(WireMessage::Client(ClientMessage::Auth { method }))
            .await
    }

    async fn hello(
        &self,
        agent: impl Into<String> + Send,
        build: BuildInfo,
    ) -> Result<(), Self::Error> {
        self.send_wire(WireMessage::Client(ClientMessage::Hello {
            agent: agent.into(),
            build,
        }))
        .await
    }

    async fn register(
        &self,
        hostname: impl Into<String> + Send,
        local_addr: impl Into<String> + Send,
        mode: IngressMode,
    ) -> Result<(), Self::Error> {
        self.send_wire(WireMessage::Client(ClientMessage::Register {
            hostname: hostname.into(),
            local_addr: local_addr.into(),
            mode,
        }))
        .await
    }

    async fn unregister(&self, hostname: impl Into<String> + Send) -> Result<(), Self::Error> {
        self.send_wire(WireMessage::Client(ClientMessage::Unregister {
            hostname: hostname.into(),
        }))
        .await
    }

    async fn http_response_start(&self, response: HttpResponseStart) -> Result<(), Self::Error> {
        self.send_wire(WireMessage::Client(ClientMessage::HttpResponseStart {
            response,
        }))
        .await
    }

    async fn http_response_body(&self, body: HttpResponseBody) -> Result<(), Self::Error> {
        self.send_wire(WireMessage::Client(ClientMessage::HttpResponseBody {
            body,
        }))
        .await
    }

    async fn tcp_stream_data(&self, data: TcpStreamData) -> Result<(), Self::Error> {
        self.send_wire(WireMessage::Client(ClientMessage::TcpStreamData { data }))
            .await
    }

    async fn tcp_stream_end(&self, stream_id: u64) -> Result<(), Self::Error> {
        self.tcp_stream_data(TcpStreamData {
            stream_id,
            chunk: ByteBuf::empty(),
            end: true,
        })
        .await
    }

    async fn pong(&self, nonce: u64) -> Result<(), Self::Error> {
        self.send_wire(WireMessage::Client(ClientMessage::Pong { nonce }))
            .await
    }
}

#[async_trait]
pub trait ServerCommands: WireSender {
    async fn auth_challenge(
        &self,
        nonce: Vec<u8>,
        oidc: Option<OidcParams>,
    ) -> Result<(), Self::Error> {
        self.send_wire(WireMessage::Server(ServerMessage::AuthChallenge {
            nonce,
            oidc,
        }))
        .await
    }

    async fn auth_ok(
        &self,
        subject: impl Into<String> + Send,
        session_id: u64,
    ) -> Result<(), Self::Error> {
        self.send_wire(WireMessage::Server(ServerMessage::AuthOk {
            subject: subject.into(),
            session_id,
        }))
        .await
    }

    async fn auth_err(&self, reason: impl Into<String> + Send) -> Result<(), Self::Error> {
        self.send_wire(WireMessage::Server(ServerMessage::AuthErr {
            reason: reason.into(),
        }))
        .await
    }

    async fn hello_ack(&self, session_id: u64, build: BuildInfo) -> Result<(), Self::Error> {
        self.send_wire(WireMessage::Server(ServerMessage::HelloAck {
            session_id,
            build,
        }))
        .await
    }

    async fn register_ok(
        &self,
        hostname: impl Into<String> + Send,
        ingress: RegisteredIngress,
    ) -> Result<(), Self::Error> {
        self.send_wire(WireMessage::Server(ServerMessage::RegisterOk {
            hostname: hostname.into(),
            ingress,
        }))
        .await
    }

    async fn register_err(
        &self,
        hostname: impl Into<String> + Send,
        reason: impl Into<String> + Send,
    ) -> Result<(), Self::Error> {
        self.send_wire(WireMessage::Server(ServerMessage::RegisterErr {
            hostname: hostname.into(),
            reason: reason.into(),
        }))
        .await
    }

    async fn http_request_start(&self, request: HttpRequestStart) -> Result<(), Self::Error> {
        self.send_wire(WireMessage::Server(ServerMessage::HttpRequestStart {
            request,
        }))
        .await
    }

    async fn http_request_body(&self, body: HttpRequestBody) -> Result<(), Self::Error> {
        self.send_wire(WireMessage::Server(ServerMessage::HttpRequestBody { body }))
            .await
    }

    async fn tcp_stream_open(&self, stream: TcpStreamOpen) -> Result<(), Self::Error> {
        self.send_wire(WireMessage::Server(ServerMessage::TcpStreamOpen { stream }))
            .await
    }

    async fn tcp_stream_data(&self, data: TcpStreamData) -> Result<(), Self::Error> {
        self.send_wire(WireMessage::Server(ServerMessage::TcpStreamData { data }))
            .await
    }

    async fn ping(&self, nonce: u64) -> Result<(), Self::Error> {
        self.send_wire(WireMessage::Server(ServerMessage::Ping { nonce }))
            .await
    }
}

impl<T: WireSender + ?Sized> ClientCommands for T {}
impl<T: WireSender + ?Sized> ServerCommands for T {}
