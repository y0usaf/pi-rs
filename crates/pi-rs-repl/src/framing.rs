//! Length-prefixed JSON framing codec shared by host and shim.
//!
//! Frame = 4-byte big-endian length + UTF-8 JSON. One writer at a time per
//! stream; the kernel manager serializes writes with a mutex because both
//! the command path and the host_response path write to the child stdin.

use std::io;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::protocol::{MAX_FRAME_BYTES, WIRE_VERSION};

/// Write one frame: 4-byte BE length + payload.
pub async fn write_frame<W: AsyncWrite + Unpin>(w: &mut W, payload: &[u8]) -> io::Result<()> {
    let len = u32::try_from(payload.len()).map_err(|_| {
        io::Error::new(io::ErrorKind::InvalidInput, "frame too large for length prefix")
    })?;
    w.write_all(&len.to_be_bytes()).await?;
    w.write_all(payload).await?;
    w.flush().await
}

/// Read one frame. Returns Ok(None) on clean EOF. Rejects over-long frames.
pub async fn read_frame<R: AsyncRead + Unpin>(r: &mut R) -> io::Result<Option<Vec<u8>>> {
    let mut len_buf = [0u8; 4];
    let mut read = 0;
    while read < 4 {
        let n = r.read(&mut len_buf[read..]).await?;
        if n == 0 {
            return if read == 0 { Ok(None) } else { Err(io::Error::new(io::ErrorKind::UnexpectedEof, "truncated length prefix")) };
        }
        read += n;
    }
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > MAX_FRAME_BYTES {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "frame exceeds max size"));
    }
    let mut buf = vec![0u8; len];
    let mut read = 0;
    while read < len {
        let n = r.read(&mut buf[read..]).await?;
        if n == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "truncated frame payload"));
        }
        read += n;
    }
    Ok(Some(buf))
}

/// Serialize a host message to a frame payload.
pub fn encode_host(msg: &crate::protocol::HostMsg) -> io::Result<Vec<u8>> {
    serde_json::to_vec(msg).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// Deserialize a shim frame payload.
pub fn decode_shim(buf: &[u8]) -> io::Result<crate::protocol::ShimMsg> {
    serde_json::from_slice(buf).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// The wire version every frame carries.
pub fn wire_version() -> u32 {
    WIRE_VERSION
}
