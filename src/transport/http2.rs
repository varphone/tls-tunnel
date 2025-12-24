// HTTP/2传输实现 - 待完成
// 由于H2库的复杂性，此功能将在后续版本中实现

use super::{Transport, TransportClient, TransportServer, TransportType};
use anyhow::Result;
use async_trait::async_trait;
use std::pin::Pin;
use tokio_rustls::{TlsAcceptor, TlsConnector};

pub struct Http2TransportClient {
    _server_addr: String,
    _server_port: u16,
    _connector: TlsConnector,
}

impl Http2TransportClient {
    pub fn new(server_addr: String, server_port: u16, connector: TlsConnector) -> Self {
        Self {
            _server_addr: server_addr,
            _server_port: server_port,
            _connector: connector,
        }
    }
}

#[async_trait]
impl TransportClient for Http2TransportClient {
    async fn connect(&self) -> Result<Pin<Box<dyn Transport>>> {
        anyhow::bail!("HTTP/2 transport not yet implemented")
    }
    
    fn transport_type(&self) -> TransportType {
        TransportType::Http2
    }
}

pub struct Http2TransportServer {
    _acceptor: TlsAcceptor,
}

impl Http2TransportServer {
    pub async fn bind(_bind_addr: String, _bind_port: u16, acceptor: TlsAcceptor) -> Result<Self> {
        Ok(Self {
            _acceptor: acceptor,
        })
    }
}

#[async_trait]
impl TransportServer for Http2TransportServer {
    async fn accept(&self) -> Result<Pin<Box<dyn Transport>>> {
        anyhow::bail!("HTTP/2 transport not yet implemented")
    }
    
    fn transport_type(&self) -> TransportType {
        TransportType::Http2
    }
}
