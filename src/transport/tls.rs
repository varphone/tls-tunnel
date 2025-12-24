use super::{Transport, TransportClient, TransportServer, TransportType};
use anyhow::{Context, Result};
use async_trait::async_trait;
use rustls::pki_types::ServerName;
use std::pin::Pin;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::{TlsAcceptor, TlsConnector};
use tracing::info;

/// TLS 传输客户端
pub struct TlsTransportClient {
    server_addr: String,
    server_port: u16,
    connector: TlsConnector,
}

impl TlsTransportClient {
    pub fn new(server_addr: String, server_port: u16, connector: TlsConnector) -> Self {
        Self {
            server_addr,
            server_port,
            connector,
        }
    }
}

#[async_trait]
impl TransportClient for TlsTransportClient {
    async fn connect(&self) -> Result<Pin<Box<dyn Transport>>> {
        let addr = format!("{}:{}", self.server_addr, self.server_port);
        info!("Connecting to {} via TLS", addr);
        
        let tcp_stream = TcpStream::connect(&addr)
            .await
            .with_context(|| format!("Failed to connect to {}", addr))?;
        
        let server_name = ServerName::try_from(self.server_addr.clone())
            .context("Invalid server name")?
            .to_owned();
        
        let tls_stream = self.connector
            .connect(server_name, tcp_stream)
            .await
            .context("TLS handshake failed")?;
        
        info!("TLS connection established to {}", addr);
        Ok(Box::pin(tls_stream))
    }
    
    fn transport_type(&self) -> TransportType {
        TransportType::Tls
    }
}

/// TLS 传输服务器
pub struct TlsTransportServer {
    listener: Arc<TcpListener>,
    acceptor: TlsAcceptor,
}

impl TlsTransportServer {
    pub async fn bind(bind_addr: String, bind_port: u16, acceptor: TlsAcceptor) -> Result<Self> {
        let addr = format!("{}:{}", bind_addr, bind_port);
        let listener = TcpListener::bind(&addr)
            .await
            .with_context(|| format!("Failed to bind to {}", addr))?;
        
        info!("TLS transport server listening on {}", addr);
        
        Ok(Self {
            listener: Arc::new(listener),
            acceptor,
        })
    }
}

#[async_trait]
impl TransportServer for TlsTransportServer {
    async fn accept(&self) -> Result<Pin<Box<dyn Transport>>> {
        let (tcp_stream, peer_addr) = self.listener
            .accept()
            .await
            .context("Failed to accept TCP connection")?;
        
        info!("Accepted TCP connection from {}", peer_addr);
        
        let tls_stream = self.acceptor
            .accept(tcp_stream)
            .await
            .context("TLS handshake failed")?;
        
        info!("TLS handshake completed with {}", peer_addr);
        Ok(Box::pin(tls_stream))
    }
    
    fn transport_type(&self) -> TransportType {
        TransportType::Tls
    }
}
