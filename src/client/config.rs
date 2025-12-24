use anyhow::Result;
use tokio::io::AsyncReadExt;

/// 环境变量前缀
pub const ENV_PREFIX: &str = "TLS_TUNNEL_";

/// 重连延迟（秒）- 可通过环境变量 TLS_TUNNEL_RECONNECT_DELAY_SECS 覆盖
pub const RECONNECT_DELAY_SECS: u64 = 5;
/// 本地服务连接重试次数 - 可通过环境变量 TLS_TUNNEL_LOCAL_CONNECT_RETRIES 覆盖
pub const LOCAL_CONNECT_RETRIES: u32 = 3;
/// 本地服务连接重试延迟（毫秒）- 可通过环境变量 TLS_TUNNEL_LOCAL_RETRY_DELAY_MS 覆盖
pub const LOCAL_RETRY_DELAY_MS: u64 = 1000;
/// 协议版本（JSON 帧）
pub const PROTOCOL_VERSION: u8 = 1;

pub fn get_reconnect_delay() -> u64 {
    std::env::var(format!("{}RECONNECT_DELAY_SECS", ENV_PREFIX))
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(RECONNECT_DELAY_SECS)
}

pub fn get_local_retries() -> u32 {
    std::env::var(format!("{}LOCAL_CONNECT_RETRIES", ENV_PREFIX))
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(LOCAL_CONNECT_RETRIES)
}

pub fn get_local_retry_delay() -> u64 {
    std::env::var(format!("{}LOCAL_RETRY_DELAY_MS", ENV_PREFIX))
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(LOCAL_RETRY_DELAY_MS)
}

/// 读取服务器返回的错误消息
pub async fn read_error_message<T>(stream: &mut T) -> Result<String>
where
    T: AsyncReadExt + Unpin,
{
    let mut msg_len_buf = [0u8; 2];
    stream.read_exact(&mut msg_len_buf).await?;
    let msg_len = u16::from_be_bytes(msg_len_buf) as usize;

    if msg_len > 4096 {
        anyhow::bail!("Error message too long");
    }

    let mut msg_buf = vec![0u8; msg_len];
    stream.read_exact(&mut msg_buf).await?;
    let message = String::from_utf8(msg_buf)?;
    Ok(message)
}
