use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
pub enum Message {
    /// Terminal I/O data
    Data(Vec<u8>),
    /// Terminal resize notification
    Resize { cols: u16, rows: u16 },
    /// Client wants to detach
    Detach,
    /// Shutdown the session
    Shutdown,
}

/// Encode a message with length-prefix framing
pub fn encode(msg: &Message) -> anyhow::Result<Vec<u8>> {
    let payload = bincode::serialize(msg)?;
    let len = (payload.len() as u32).to_be_bytes();
    let mut buf = Vec::with_capacity(4 + payload.len());
    buf.extend_from_slice(&len);
    buf.extend_from_slice(&payload);
    Ok(buf)
}

/// Read a length-prefixed message from a reader
pub async fn decode(reader: &mut (impl tokio::io::AsyncReadExt + Unpin)) -> anyhow::Result<Message> {
    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;

    let mut payload = vec![0u8; len];
    reader.read_exact(&mut payload).await?;

    let msg = bincode::deserialize(&payload)?;
    Ok(msg)
}
