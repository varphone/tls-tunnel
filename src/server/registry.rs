use crate::config::ProxyType;
use crate::stats::ProxyStatsTracker;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};

/// 代理配置信息（从客户端接收）
#[derive(Debug, Clone)]
pub struct ProxyInfo {
    pub name: String,
    pub proxy_type: ProxyType,
    pub publish_addr: String,
    pub publish_port: u16,
    pub local_port: u16,
}

/// Visitor 配置信息（从客户端接收）
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct VisitorInfo {
    pub name: String,
    pub bind_addr: String,
    pub bind_port: u16,
    pub publish_port: u16,
}

/// 全局代理注册表项
#[derive(Clone)]
pub struct ProxyRegistration {
    /// 用于请求该客户端创建新stream的channel
    pub stream_tx: mpsc::Sender<(mpsc::Sender<yamux::Stream>, u16, String)>,
    /// 代理信息
    pub proxy_info: ProxyInfo,
}

/// 全局代理注册表，维护 (proxy_name, publish_port) -> ProxyRegistration 的映射
pub type ProxyRegistry = Arc<RwLock<HashMap<(String, u16), ProxyRegistration>>>;

/// RAII guard to automatically decrement active connections count
pub struct ConnectionGuard {
    tracker: ProxyStatsTracker,
}

impl ConnectionGuard {
    pub fn new(tracker: ProxyStatsTracker) -> Self {
        Self { tracker }
    }
}

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        self.tracker.connection_ended();
    }
}
