/// TLS Tunnel 库入口
///
/// 将核心模块导出为库，方便测试和复用
pub mod cli;
pub mod client;
pub mod config;
pub mod connection_pool;
pub mod error;
pub mod io_util;
pub mod limited_reader;
pub mod protocol;
pub mod rate_limiter;
pub mod server;
pub mod stats;
pub mod tls;
pub mod top;
pub mod transport;

// 重新导出常用类型
pub use client::{ForwarderHandler, HandlerStatus, ProxyHandler, ProxyManager, VisitorHandler};
pub use config::{AppConfig, ClientConfig, ServerConfig};
pub use error::{Result, TunnelError};
pub use io_util::{write_vectored_all, VecBuffer};
pub use limited_reader::{LimitedReader, DEFAULT_MAX_HEADER_SIZE, DEFAULT_MAX_REQUEST_SIZE};
pub use rate_limiter::{RateLimiter, RateLimiterConfig};
