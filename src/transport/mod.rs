mod factory;
mod http2;
mod tls;
mod wss;

pub use factory::{create_transport_client, create_transport_server};
pub use http2::{Http2TransportClient, Http2TransportServer};
pub use tls::{TlsTransportClient, TlsTransportServer};
pub use wss::{WssTransportClient, WssTransportServer};

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::pin::Pin;
use tokio::io::{AsyncRead, AsyncWrite};

/// 传输层类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum TransportType {
    /// TCP + TLS（原生方式）
    #[default]
    Tls,
    /// HTTP/2.0 over TLS
    #[serde(rename = "http2")]
    Http2,
    /// WebSocket Secure
    Wss,
}

impl std::fmt::Display for TransportType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TransportType::Tls => write!(f, "tls"),
            TransportType::Http2 => write!(f, "http2"),
            TransportType::Wss => write!(f, "wss"),
        }
    }
}

impl std::str::FromStr for TransportType {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "tls" => Ok(Self::Tls),
            "http2" | "h2" => Ok(Self::Http2),
            "wss" | "websocket" => Ok(Self::Wss),
            _ => anyhow::bail!("Unknown transport type: {}", s),
        }
    }
}

/// 传输层连接抽象
///
/// 统一封装不同传输方式（TLS、HTTP/2、WebSocket）的连接
pub trait Transport: AsyncRead + AsyncWrite + Unpin + Send + 'static {}

// 为所有满足条件的类型自动实现 Transport
impl<T> Transport for T where T: AsyncRead + AsyncWrite + Unpin + Send + 'static {}

/// 传输层客户端接口
#[async_trait]
pub trait TransportClient: Send + Sync {
    /// 连接到服务器并返回传输层连接
    async fn connect(&self) -> Result<Pin<Box<dyn Transport>>>;

    /// 获取传输类型
    fn transport_type(&self) -> TransportType;
}

/// 传输层服务器接口
#[async_trait]
pub trait TransportServer: Send + Sync {
    /// 接受新的传输层连接
    async fn accept(&self) -> Result<Pin<Box<dyn Transport>>>;

    /// 获取传输类型
    fn transport_type(&self) -> TransportType;
}
