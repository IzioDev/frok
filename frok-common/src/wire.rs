use std::io;

use frok_protocol::{MAX_FRAME_LEN, WireMessage, decode_frame, encode_frame};
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

#[derive(Debug, Error)]
pub enum ReadWireError {
    #[error("io error: {0}")]
    Io(#[source] io::Error),
    #[error("frame exceeds max size: {len} > {max}")]
    FrameTooLarge { len: u32, max: u32 },
    #[error("decode error: {0}")]
    Decode(#[source] io::Error),
}

pub async fn read_wire_message<R: AsyncRead + Unpin>(
    reader: &mut R,
    buffer: &mut Vec<u8>,
) -> Result<WireMessage, ReadWireError> {
    let mut len_bytes = [0u8; 4];
    if let Err(err) = reader.read_exact(&mut len_bytes).await {
        return Err(ReadWireError::Io(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            err,
        )));
    }

    let len = u32::from_le_bytes(len_bytes);
    if len > MAX_FRAME_LEN {
        return Err(ReadWireError::FrameTooLarge {
            len,
            max: MAX_FRAME_LEN,
        });
    }

    buffer.resize(len as usize, 0);
    if let Err(err) = reader.read_exact(buffer.as_mut_slice()).await {
        return Err(ReadWireError::Io(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            err,
        )));
    }

    decode_frame(buffer).map_err(ReadWireError::Decode)
}

pub async fn write_wire_message<W: AsyncWrite + Unpin>(
    writer: &mut W,
    message: &WireMessage,
) -> io::Result<()> {
    let frame = encode_frame(message)?;
    writer.write_all(&frame).await?;
    Ok(())
}
