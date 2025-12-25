use crate::config::{RoutingConfig, RoutingStrategy};
use anyhow::Result;
use maxminddb::{geoip2, MaxMindDBError, Reader};
use std::net::{IpAddr, ToSocketAddrs};
use std::sync::Arc;
use tracing::{debug, info, warn};

/// GeoIP 路由器
pub struct GeoIpRouter {
    reader: Option<Arc<Reader<Vec<u8>>>>,
    config: RoutingConfig,
}

impl GeoIpRouter {
    /// 创建新的 GeoIP 路由器
    pub fn new(config: RoutingConfig) -> Result<Self> {
        let reader = if let Some(ref db_path) = config.geoip_db {
            match Reader::open_readfile(db_path) {
                Ok(reader) => {
                    info!("GeoIP database loaded from: {}", db_path);
                    Some(Arc::new(reader))
                }
                Err(e) => {
                    warn!("Failed to load GeoIP database from {}: {}", db_path, e);
                    warn!("Routing will use default strategy for all addresses");
                    None
                }
            }
        } else {
            debug!("No GeoIP database configured, routing will use default strategy");
            None
        };

        Ok(Self { reader, config })
    }

    /// 判断目标地址是否应该直连
    pub fn should_direct_connect(&self, target: &str) -> bool {
        // 解析目标地址，提取主机名或 IP
        let host = if let Some(colon_pos) = target.rfind(':') {
            &target[..colon_pos]
        } else {
            target
        };

        // 尝试解析为 IP 地址
        if let Ok(ip) = host.parse::<IpAddr>() {
            return self.should_direct_connect_ip(ip);
        }

        // 如果是域名，尝试解析
        if let Ok(addrs) = (host, 0).to_socket_addrs() {
            for addr in addrs {
                let ip = addr.ip();
                // 只要有一个 IP 符合直连条件就直连
                if self.should_direct_connect_ip(ip) {
                    debug!("Domain {} resolved to {}, using direct connection", host, ip);
                    return true;
                }
            }
        }

        // 无法解析或没有符合条件的 IP，使用默认策略
        debug!(
            "Cannot resolve {} to IP, using default strategy: {:?}",
            host, self.config.default_strategy
        );
        self.config.default_strategy == RoutingStrategy::Direct
    }

    /// 判断 IP 地址是否应该直连
    fn should_direct_connect_ip(&self, ip: IpAddr) -> bool {
        // 如果没有 GeoIP 数据库，使用默认策略
        let Some(ref reader) = self.reader else {
            return self.config.default_strategy == RoutingStrategy::Direct;
        };

        // 查询 IP 的国家代码
        let country_code = match self.lookup_country(reader, ip) {
            Ok(Some(code)) => code,
            Ok(None) => {
                debug!("No country found for IP {}, using default strategy", ip);
                return self.config.default_strategy == RoutingStrategy::Direct;
            }
            Err(e) => {
                warn!("Failed to lookup IP {}: {}, using default strategy", ip, e);
                return self.config.default_strategy == RoutingStrategy::Direct;
            }
        };

        debug!("IP {} is from country: {}", ip, country_code);

        // 检查是否在直连列表中
        if self.config.direct_countries.contains(&country_code) {
            debug!(
                "Country {} is in direct_countries list, using direct connection",
                country_code
            );
            return true;
        }

        // 检查是否在代理列表中
        if !self.config.proxy_countries.is_empty()
            && self.config.proxy_countries.contains(&country_code)
        {
            debug!(
                "Country {} is in proxy_countries list, using proxy",
                country_code
            );
            return false;
        }

        // 不在任何列表中，使用默认策略
        debug!(
            "Country {} not in any list, using default strategy: {:?}",
            country_code, self.config.default_strategy
        );
        self.config.default_strategy == RoutingStrategy::Direct
    }

    /// 查询 IP 地址的国家代码
    fn lookup_country(
        &self,
        reader: &Reader<Vec<u8>>,
        ip: IpAddr,
    ) -> Result<Option<String>, MaxMindDBError> {
        let country: geoip2::Country = reader.lookup(ip)?;

        Ok(country
            .country
            .and_then(|c| c.iso_code)
            .map(|s| s.to_uppercase()))
    }
}

impl std::fmt::Debug for GeoIpRouter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GeoIpRouter")
            .field("has_reader", &self.reader.is_some())
            .field("config", &self.config)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_strategy() {
        let config = RoutingConfig {
            geoip_db: None,
            direct_countries: vec![],
            proxy_countries: vec![],
            default_strategy: RoutingStrategy::Direct,
        };

        let router = GeoIpRouter::new(config).unwrap();
        // 没有数据库时应该使用默认策略
        assert!(router.should_direct_connect("8.8.8.8:80"));
    }
}
