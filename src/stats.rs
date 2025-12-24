use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

/// Statistics for a single proxy
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyStats {
    /// Proxy name
    pub name: String,
    /// Server published address
    pub publish_addr: String,
    /// Server published port
    pub publish_port: u16,
    /// Client local port
    pub local_port: u16,
    /// Total number of connections
    pub total_connections: u64,
    /// Currently active connections
    pub active_connections: u64,
    /// Total bytes sent to client
    pub bytes_sent: u64,
    /// Total bytes received from client
    pub bytes_received: u64,
    /// Timestamp when this proxy was registered (Unix timestamp)
    pub start_time: u64,
}

/// Statistics tracker for a single proxy
#[derive(Debug, Clone)]
pub struct ProxyStatsTracker {
    name: String,
    publish_addr: String,
    publish_port: u16,
    local_port: u16,
    total_connections: Arc<AtomicU64>,
    active_connections: Arc<AtomicU64>,
    bytes_sent: Arc<AtomicU64>,
    bytes_received: Arc<AtomicU64>,
    start_time: u64,
}

impl ProxyStatsTracker {
    pub fn new(name: String, publish_addr: String, publish_port: u16, local_port: u16) -> Self {
        Self {
            name,
            publish_addr,
            publish_port,
            local_port,
            total_connections: Arc::new(AtomicU64::new(0)),
            active_connections: Arc::new(AtomicU64::new(0)),
            bytes_sent: Arc::new(AtomicU64::new(0)),
            bytes_received: Arc::new(AtomicU64::new(0)),
            start_time: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        }
    }

    /// Increment active connections (called when connection starts)
    pub fn connection_started(&self) {
        self.total_connections.fetch_add(1, Ordering::Relaxed);
        self.active_connections.fetch_add(1, Ordering::Relaxed);
    }

    /// Decrement active connections (called when connection ends)
    pub fn connection_ended(&self) {
        self.active_connections.fetch_sub(1, Ordering::Relaxed);
    }

    /// Add bytes sent
    pub fn add_bytes_sent(&self, bytes: u64) {
        self.bytes_sent.fetch_add(bytes, Ordering::Relaxed);
    }

    /// Add bytes received
    pub fn add_bytes_received(&self, bytes: u64) {
        self.bytes_received.fetch_add(bytes, Ordering::Relaxed);
    }

    /// Get current snapshot of stats
    pub fn get_stats(&self) -> ProxyStats {
        ProxyStats {
            name: self.name.clone(),
            publish_addr: self.publish_addr.clone(),
            publish_port: self.publish_port,
            local_port: self.local_port,
            total_connections: self.total_connections.load(Ordering::Relaxed),
            active_connections: self.active_connections.load(Ordering::Relaxed),
            bytes_sent: self.bytes_sent.load(Ordering::Relaxed),
            bytes_received: self.bytes_received.load(Ordering::Relaxed),
            start_time: self.start_time,
        }
    }
}

/// Global statistics manager
#[derive(Debug, Clone)]
pub struct StatsManager {
    proxies: Arc<Mutex<HashMap<String, ProxyStatsTracker>>>,
}

impl StatsManager {
    pub fn new() -> Self {
        Self {
            proxies: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Register a new proxy
    pub fn register_proxy(
        &self,
        name: String,
        publish_addr: String,
        publish_port: u16,
        local_port: u16,
    ) -> ProxyStatsTracker {
        let tracker = ProxyStatsTracker::new(name.clone(), publish_addr, publish_port, local_port);
        self.proxies
            .lock()
            .unwrap()
            .insert(name, tracker.clone());
        tracker
    }

    /// Unregister a proxy
    pub fn unregister_proxy(&self, name: &str) {
        self.proxies.lock().unwrap().remove(name);
    }

    /// Get stats for all proxies
    pub fn get_all_stats(&self) -> Vec<ProxyStats> {
        self.proxies
            .lock()
            .unwrap()
            .values()
            .map(|tracker| tracker.get_stats())
            .collect()
    }

    /// Get stats for a specific proxy
    pub fn get_proxy_stats(&self, name: &str) -> Option<ProxyStats> {
        self.proxies
            .lock()
            .unwrap()
            .get(name)
            .map(|tracker| tracker.get_stats())
    }

    /// Clear all stats
    pub fn clear(&self) {
        self.proxies.lock().unwrap().clear();
    }
}

impl Default for StatsManager {
    fn default() -> Self {
        Self::new()
    }
}
