use bytes::Bytes;

#[derive(Debug)]
pub enum TcpStreamEvent {
    Data(Bytes),
    End,
}
