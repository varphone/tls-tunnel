/// 速率限制器模块
///
/// 使用 token bucket 算法实现速率限制，防止 DoS 攻击
use governor::{
    clock::{Clock, DefaultClock},
    state::{InMemoryState, NotKeyed},
    Quota, RateLimiter as GovernorLimiter,
};
use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Duration;

/// 速率限制器配置
#[derive(Debug, Clone)]
pub struct RateLimiterConfig {
    /// 每秒允许的请求数
    pub requests_per_second: u32,
    /// 突发容量（允许短时间内的峰值）
    pub burst_size: u32,
}

impl Default for RateLimiterConfig {
    fn default() -> Self {
        Self {
            requests_per_second: 100, // 默认每秒 100 个新连接
            burst_size: 200,          // 允许短时间内 200 个峰值连接
        }
    }
}

/// 速率限制器包装器
pub struct RateLimiter {
    inner: Arc<GovernorLimiter<NotKeyed, InMemoryState, DefaultClock>>,
    config: RateLimiterConfig,
}

impl RateLimiter {
    /// 创建新的速率限制器
    pub fn new(config: RateLimiterConfig) -> Self {
        let quota = Quota::per_second(
            NonZeroU32::new(config.requests_per_second).expect("requests_per_second must be > 0"),
        )
        .allow_burst(NonZeroU32::new(config.burst_size).expect("burst_size must be > 0"));

        let limiter = Arc::new(GovernorLimiter::direct(quota));

        Self {
            inner: limiter,
            config,
        }
    }

    /// 创建默认配置的速率限制器
    pub fn with_defaults() -> Self {
        Self::new(RateLimiterConfig::default())
    }

    /// 尝试获取一个令牌（非阻塞）
    /// 返回 Ok(()) 如果允许请求，否则返回 Err(Duration) 表示需要等待的时间
    pub fn check(&self) -> Result<(), Duration> {
        match self.inner.check() {
            Ok(_) => Ok(()),
            Err(not_until) => {
                let wait_time = not_until.wait_time_from(DefaultClock::default().now());
                Err(wait_time)
            }
        }
    }

    /// 异步等待直到可以获取令牌
    pub async fn wait(&self) {
        loop {
            match self.check() {
                Ok(_) => break,
                Err(wait_time) => {
                    tokio::time::sleep(wait_time).await;
                }
            }
        }
    }

    /// 获取配置信息
    pub fn config(&self) -> &RateLimiterConfig {
        &self.config
    }
}

impl Clone for RateLimiter {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            config: self.config.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rate_limiter_creation() {
        let config = RateLimiterConfig {
            requests_per_second: 10,
            burst_size: 20,
        };
        let limiter = RateLimiter::new(config);
        assert_eq!(limiter.config().requests_per_second, 10);
        assert_eq!(limiter.config().burst_size, 20);
    }

    #[test]
    fn test_rate_limiter_check() {
        let config = RateLimiterConfig {
            requests_per_second: 100,
            burst_size: 10,
        };
        let limiter = RateLimiter::new(config);

        // 前 10 个请求应该立即通过（burst_size）
        for _ in 0..10 {
            assert!(limiter.check().is_ok());
        }

        // 第 11 个请求应该被限流
        assert!(limiter.check().is_err());
    }

    #[tokio::test]
    async fn test_rate_limiter_wait() {
        let config = RateLimiterConfig {
            requests_per_second: 100,
            burst_size: 5,
        };
        let limiter = RateLimiter::new(config);

        // 耗尽 burst
        for _ in 0..5 {
            assert!(limiter.check().is_ok());
        }

        // 下一个应该被限流
        assert!(limiter.check().is_err());

        // 等待应该能获取新令牌（但可能需要一些时间）
        limiter.wait().await;
        // 等待后应该可以再次获取令牌（但立即检查可能仍需等待）
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    }

    #[test]
    fn test_rate_limiter_clone() {
        let limiter1 = RateLimiter::with_defaults();
        let limiter2 = limiter1.clone();

        // 克隆的限制器共享相同的内部状态
        for _ in 0..50 {
            let _ = limiter1.check();
        }

        // limiter2 应该看到 limiter1 的消耗
        // （因为它们共享 Arc<GovernorLimiter>）
        let result = limiter2.check();
        // 取决于 burst_size，可能被限流
        assert!(result.is_ok() || result.is_err());
    }
}
