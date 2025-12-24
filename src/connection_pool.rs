use anyhow::{Context, Result};
use socket2::{SockRef, TcpKeepalive};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

/// 连接池配置
#[derive(Debug, Clone)]
pub struct PoolConfig {
    /// 最小空闲连接数
    pub min_idle: usize,
    /// 最大连接数
    pub max_size: usize,
    /// 连接最大空闲时间（秒）
    pub max_idle_time: Duration,
    /// 连接建立超时（毫秒）
    pub connect_timeout: Duration,
    /// Keepalive 首次探测时间
    pub keepalive_time: Option<Duration>,
    /// Keepalive 探测间隔
    pub keepalive_interval: Option<Duration>,
    /// 是否复用连接（false 时每次创建新连接，适用于 TCP/HTTP/1.1 短连接）
    pub reuse_connections: bool,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            min_idle: 2,
            max_size: 10,
            max_idle_time: Duration::from_secs(60),
            connect_timeout: Duration::from_millis(5000),
            keepalive_time: Some(Duration::from_secs(30)),
            keepalive_interval: Some(Duration::from_secs(10)),
            reuse_connections: false, // 默认不复用，避免 HTTP/1.1 问题
        }
    }
}

/// 池中的连接
struct PooledConnection {
    stream: TcpStream,
    #[allow(dead_code)]
    created_at: Instant,
    last_used: Instant,
}

impl PooledConnection {
    fn new(stream: TcpStream) -> Self {
        let now = Instant::now();
        Self {
            stream,
            created_at: now,
            last_used: now,
        }
    }

    fn is_expired(&self, max_idle_time: Duration) -> bool {
        self.last_used.elapsed() > max_idle_time
    }

    fn update_last_used(&mut self) {
        self.last_used = Instant::now();
    }
}

/// 单个地址的连接池
struct AddressPool {
    address: String,
    config: PoolConfig,
    idle_connections: Vec<PooledConnection>,
    active_count: usize,
}

impl AddressPool {
    fn new(address: String, config: PoolConfig) -> Self {
        Self {
            address,
            config,
            idle_connections: Vec::new(),
            active_count: 0,
        }
    }

    /// 获取连接
    async fn get_connection(&mut self) -> Result<TcpStream> {
        // 清理过期连接
        self.cleanup_expired();

        // 尝试从空闲连接中获取
        if let Some(mut pooled) = self.idle_connections.pop() {
            pooled.update_last_used();
            self.active_count += 1;
            debug!(
                "Reusing pooled connection to {} (active: {}, idle: {})",
                self.address,
                self.active_count,
                self.idle_connections.len()
            );
            return Ok(pooled.stream);
        }

        // 检查是否达到最大连接数
        let total_connections = self.active_count + self.idle_connections.len();
        if total_connections >= self.config.max_size {
            anyhow::bail!(
                "Connection pool exhausted for {} (max: {})",
                self.address,
                self.config.max_size
            );
        }

        // 创建新连接
        debug!(
            "Creating new connection to {} (active: {}, idle: {})",
            self.address,
            self.active_count,
            self.idle_connections.len()
        );

        let stream = tokio::time::timeout(
            self.config.connect_timeout,
            TcpStream::connect(&self.address),
        )
        .await
        .context("Connection timeout")?
        .context("Failed to connect")?;

        apply_keepalive(&stream, &self.config);

        self.active_count += 1;
        Ok(stream)
    }

    fn return_connection(&mut self, stream: TcpStream) {
        self.active_count = self.active_count.saturating_sub(1);

        // 如果配置不允许复用，直接丢弃
        if !self.config.reuse_connections {
            debug!("Connection reuse disabled, discarding connection to {}", self.address);
            return;
        }

        // 检查连接健康状态
        if !is_connection_healthy(&stream) {
            debug!("Connection to {} is unhealthy, discarding", self.address);
            return;
        }

        // 检查是否应该保留这个连接
        let total_idle = self.idle_connections.len();
        if total_idle >= self.config.max_size - self.active_count {
            debug!(
                "Dropping connection to {} (pool full, idle: {})",
                self.address, total_idle
            );
            return;
        }

        debug!(
            "Returning connection to pool for {} (active: {}, idle: {})",
            self.address, self.active_count, total_idle
        );

        self.idle_connections.push(PooledConnection::new(stream));
    }

    /// 丢弃不可用的连接，同时修正计数
    fn discard_connection(&mut self, _stream: TcpStream) {
        self.active_count = self.active_count.saturating_sub(1);
        debug!("Discarded bad connection to {}", self.address);
    }

    /// 清理过期的空闲连接
    fn cleanup_expired(&mut self) {
        let before = self.idle_connections.len();
        self.idle_connections
            .retain(|conn| !conn.is_expired(self.config.max_idle_time));
        let removed = before - self.idle_connections.len();

        if removed > 0 {
            debug!(
                "Cleaned up {} expired connections to {}",
                removed, self.address
            );
        }
    }

    /// 预热连接池
    async fn warmup(&mut self) -> Result<()> {
        let target = self
            .config
            .min_idle
            .saturating_sub(self.idle_connections.len());

        if target == 0 {
            return Ok(());
        }

        info!("Warming up {} connections to {}", target, self.address);

        for _ in 0..target {
            match tokio::time::timeout(
                self.config.connect_timeout,
                TcpStream::connect(&self.address),
            )
            .await
            {
                Ok(Ok(stream)) => {
                    apply_keepalive(&stream, &self.config);
                    self.idle_connections.push(PooledConnection::new(stream));
                }
                Ok(Err(e)) => {
                    warn!("Failed to warm up connection to {}: {}", self.address, e);
                }
                Err(_) => {
                    warn!("Timeout warming up connection to {}", self.address);
                }
            }
        }

        info!(
            "Warmed up {} connections to {} (target: {})",
            self.idle_connections.len(),
            self.address,
            target
        );

        Ok(())
    }

    /// 获取池的统计信息
    #[allow(dead_code)]
    fn stats(&self) -> PoolStats {
        PoolStats {
            active: self.active_count,
            idle: self.idle_connections.len(),
            total: self.active_count + self.idle_connections.len(),
            max_size: self.config.max_size,
        }
    }
}

fn apply_keepalive(stream: &TcpStream, config: &PoolConfig) {
    if config.keepalive_time.is_none() && config.keepalive_interval.is_none() {
        return;
    }

    let mut keepalive = TcpKeepalive::new();
    if let Some(time) = config.keepalive_time {
        keepalive = keepalive.with_time(time);
    }
    if let Some(interval) = config.keepalive_interval {
        keepalive = keepalive.with_interval(interval);
    }

    let sock_ref = SockRef::from(stream);
    if let Err(e) = sock_ref.set_tcp_keepalive(&keepalive) {
        warn!(
            "Failed to set TCP keepalive on {}: {}",
            stream
                .peer_addr()
                .map(|a| a.to_string())
                .unwrap_or_else(|_| "unknown".into()),
            e
        );
    }
}

/// 检查连接是否健康（未被远端关闭、无错误）
fn is_connection_healthy(stream: &TcpStream) -> bool {
    // 使用 try_read 来检查连接状态而不消耗数据
    let mut buf = [0u8; 1];
    match stream.try_read(&mut buf) {
        Ok(0) => false, // EOF，连接已关闭
        Ok(_) => {
            // 不应该有数据，因为这是空闲连接
            // 但如果有数据，连接仍然是活的
            warn!("Unexpected data in idle connection");
            true
        },
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => true, // 无数据，连接正常
        Err(e) => {
            // 其他错误，连接不健康
            debug!("Connection health check failed: {}", e);
            false
        }
    }
}

/// 连接池统计信息
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct PoolStats {
    pub active: usize,
    pub idle: usize,
    pub total: usize,
    pub max_size: usize,
}

/// 连接池管理器
pub struct ConnectionPool {
    pools: Arc<Mutex<HashMap<String, AddressPool>>>,
    config: PoolConfig,
}

impl ConnectionPool {
    /// 创建新的连接池
    pub fn new(config: PoolConfig) -> Self {
        Self {
            pools: Arc::new(Mutex::new(HashMap::new())),
            config,
        }
    }

    /// 使用默认配置创建连接池
    #[allow(dead_code)]
    pub fn with_defaults() -> Self {
        Self::new(PoolConfig::default())
    }

    /// 获取连接
    pub async fn get(&self, address: &str) -> Result<TcpStream> {
        let mut pools = self.pools.lock().await;

        let pool = pools
            .entry(address.to_string())
            .or_insert_with(|| AddressPool::new(address.to_string(), self.config.clone()));

        pool.get_connection().await
    }

    /// 归还连接（如果连接仍然可用）
    #[allow(dead_code)]
    pub async fn return_connection(&self, address: &str, stream: TcpStream) {
        let mut pools = self.pools.lock().await;

        if let Some(pool) = pools.get_mut(address) {
            pool.return_connection(stream);
        }
    }

    /// 丢弃不可用的连接（例如读写错误后）
    pub async fn discard_connection(&self, address: &str, stream: TcpStream) {
        let mut pools = self.pools.lock().await;

        if let Some(pool) = pools.get_mut(address) {
            pool.discard_connection(stream);
        }
    }

    /// 预热指定地址的连接池
    pub async fn warmup(&self, address: &str) -> Result<()> {
        let mut pools = self.pools.lock().await;

        let pool = pools
            .entry(address.to_string())
            .or_insert_with(|| AddressPool::new(address.to_string(), self.config.clone()));

        pool.warmup().await
    }

    /// 预热多个地址的连接池
    pub async fn warmup_all(&self, addresses: &[String]) -> Result<()> {
        for address in addresses {
            if let Err(e) = self.warmup(address).await {
                warn!("Failed to warm up pool for {}: {}", address, e);
            }
        }
        Ok(())
    }

    /// 获取指定地址的池统计信息
    #[allow(dead_code)]
    pub async fn stats(&self, address: &str) -> Option<PoolStats> {
        let pools = self.pools.lock().await;
        pools.get(address).map(|pool| pool.stats())
    }

    /// 获取所有池的统计信息
    #[allow(dead_code)]
    pub async fn all_stats(&self) -> HashMap<String, PoolStats> {
        let pools = self.pools.lock().await;
        pools
            .iter()
            .map(|(addr, pool)| (addr.clone(), pool.stats()))
            .collect()
    }

    /// 清理所有过期连接
    pub async fn cleanup_expired(&self) {
        let mut pools = self.pools.lock().await;
        for pool in pools.values_mut() {
            pool.cleanup_expired();
        }
    }

    /// 启动后台清理任务
    pub fn start_cleanup_task(self: Arc<Self>, interval: Duration) {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(interval);
            loop {
                interval.tick().await;
                self.cleanup_expired().await;
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;

    #[tokio::test]
    async fn test_pool_creation() {
        let pool = ConnectionPool::with_defaults();
        assert!(pool.all_stats().await.is_empty());
    }

    fn make_test_stream() -> TcpStream {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        thread::spawn(move || {
            let _ = std::net::TcpStream::connect(addr);
        });
        let (stream, _) = listener.accept().unwrap();
        TcpStream::from_std(stream).unwrap()
    }

    #[test]
    fn test_pooled_connection_expiry() {
        let conn = PooledConnection {
            stream: make_test_stream(),
            created_at: Instant::now(),
            last_used: Instant::now() - Duration::from_secs(120),
        };

        assert!(conn.is_expired(Duration::from_secs(60)));
        assert!(!conn.is_expired(Duration::from_secs(180)));
    }
}
