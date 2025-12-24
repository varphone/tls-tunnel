// 传输层工厂 - 根据配置创建传输实例

use crate::config::{ClientConfig, ServerConfig};
use crate::transport::{
    Http2TransportClient, Http2TransportServer, TlsTransportClient, TlsTransportServer,
    TransportClient, TransportServer, TransportType, WssTransportClient, WssTransportServer,
};
use anyhow::{Context, Result};
use std::sync::Arc;
use tokio_rustls::{TlsAcceptor, TlsConnector};

/// 创建传输层客户端
pub fn create_transport_client(
    config: &ClientConfig,
    connector: TlsConnector,
) -> Result<Arc<dyn TransportClient>> {
    let client: Arc<dyn TransportClient> = match config.transport {
        TransportType::Tls => Arc::new(TlsTransportClient::new(
            config.server_addr.clone(),
            config.server_port,
            connector,
        )),
        TransportType::Http2 => Arc::new(Http2TransportClient::new(
            config.server_addr.clone(),
            config.server_port,
            config.server_path.clone(),
            connector,
        )),
        TransportType::Wss => Arc::new(WssTransportClient::new(
            config.server_addr.clone(),
            config.server_port,
            config.server_path.clone(),
            connector,
        )),
    };

    Ok(client)
}

/// 创建传输层服务器
pub async fn create_transport_server(
    config: &ServerConfig,
    acceptor: TlsAcceptor,
) -> Result<Arc<dyn TransportServer>> {
    let server: Arc<dyn TransportServer> = match config.transport {
        TransportType::Tls => {
            let server =
                TlsTransportServer::bind(config.bind_addr.clone(), config.bind_port, acceptor)
                    .await
                    .context("Failed to bind TLS transport server")?;
            Arc::new(server)
        }
        TransportType::Http2 => {
            let server = Http2TransportServer::bind(
                config.bind_addr.clone(),
                config.bind_port,
                acceptor,
                config.behind_proxy,
            )
            .await
            .context("Failed to bind HTTP/2 transport server")?;
            Arc::new(server)
        }
        TransportType::Wss => {
            let server = WssTransportServer::bind(
                config.bind_addr.clone(),
                config.bind_port,
                acceptor,
                config.behind_proxy,
            )
            .await
            .context("Failed to bind WebSocket transport server")?;
            Arc::new(server)
        }
    };

    Ok(server)
}
