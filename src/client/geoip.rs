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
    direct_networks: Vec<ipnetwork::IpNetwork>,
    proxy_networks: Vec<ipnetwork::IpNetwork>,
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

        // 解析直连 IP/CIDR 列表
        let mut direct_networks = Vec::new();
        for ip_str in &config.direct_ips {
            match ip_str.parse::<ipnetwork::IpNetwork>() {
                Ok(network) => {
                    debug!("Added direct IP/CIDR: {}", network);
                    direct_networks.push(network);
                }
                Err(e) => {
                    warn!("Failed to parse direct IP/CIDR '{}': {}", ip_str, e);
                }
            }
        }

        // 解析代理 IP/CIDR 列表
        let mut proxy_networks = Vec::new();
        for ip_str in &config.proxy_ips {
            match ip_str.parse::<ipnetwork::IpNetwork>() {
                Ok(network) => {
                    debug!("Added proxy IP/CIDR: {}", network);
                    proxy_networks.push(network);
                }
                Err(e) => {
                    warn!("Failed to parse proxy IP/CIDR '{}': {}", ip_str, e);
                }
            }
        }

        Ok(Self {
            reader,
            config,
            direct_networks,
            proxy_networks,
        })
    }

    /// 判断目标地址是否应该直连
    pub fn should_direct_connect(&self, target: &str) -> bool {
        // 解析目标地址，提取主机名或 IP
        let host = if let Some(colon_pos) = target.rfind(':') {
            &target[..colon_pos]
        } else {
            target
        };

        // 1. 检查域名匹配（优先级最高）
        if let Some(should_direct) = self.match_domain(host) {
            debug!(
                "Domain {} matched in routing rules, using {}",
                host,
                if should_direct { "direct" } else { "proxy" }
            );
            return should_direct;
        }

        // 2. 尝试解析为 IP 地址
        if let Ok(ip) = host.parse::<IpAddr>() {
            return self.should_direct_connect_ip(ip);
        }

        // 3. 如果是域名，尝试解析
        if let Ok(addrs) = (host, 0).to_socket_addrs() {
            for addr in addrs {
                let ip = addr.ip();
                // 只要有一个 IP 符合直连条件就直连
                if self.should_direct_connect_ip(ip) {
                    debug!(
                        "Domain {} resolved to {}, using direct connection",
                        host, ip
                    );
                    return true;
                }
            }
        }

        // 4. 无法解析或没有符合条件的 IP，使用默认策略
        debug!(
            "Cannot resolve {} to IP, using default strategy: {:?}",
            host, self.config.default_strategy
        );
        self.config.default_strategy == RoutingStrategy::Direct
    }

    /// 检查域名是否匹配路由规则
    /// 返回 Some(true) 表示应该直连，Some(false) 表示应该代理，None 表示未匹配
    fn match_domain(&self, host: &str) -> Option<bool> {
        // 检查直连域名列表
        for pattern in &self.config.direct_domains {
            if Self::domain_matches(host, pattern) {
                return Some(true);
            }
        }

        // 检查代理域名列表
        for pattern in &self.config.proxy_domains {
            if Self::domain_matches(host, pattern) {
                return Some(false);
            }
        }

        None
    }

    /// 检查域名是否匹配模式（支持通配符）
    fn domain_matches(domain: &str, pattern: &str) -> bool {
        if let Some(suffix) = pattern.strip_prefix("*.") {
            // 通配符匹配：*.example.com 匹配 www.example.com、api.example.com
            domain.ends_with(suffix) || domain == &suffix[1..] // 也匹配 example.com
        } else if let Some(suffix) = pattern.strip_prefix('.') {
            // .example.com 匹配 www.example.com 但不匹配 example.com
            domain.ends_with(pattern) || domain == suffix
        } else {
            // 精确匹配
            domain == pattern
        }
    }

    /// 判断 IP 地址是否应该直连
    fn should_direct_connect_ip(&self, ip: IpAddr) -> bool {
        // 1. 检查是否在直连 IP/CIDR 列表中
        for network in &self.direct_networks {
            if network.contains(ip) {
                debug!(
                    "IP {} matched direct network {}, using direct connection",
                    ip, network
                );
                return true;
            }
        }

        // 2. 检查是否在代理 IP/CIDR 列表中
        for network in &self.proxy_networks {
            if network.contains(ip) {
                debug!("IP {} matched proxy network {}, using proxy", ip, network);
                return false;
            }
        }

        // 3. 如果没有 GeoIP 数据库，使用默认策略
        let Some(ref reader) = self.reader else {
            return self.config.default_strategy == RoutingStrategy::Direct;
        };

        // 4. 查询 IP 的国家代码
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

        // 5. 检查是否在直连国家列表中
        if self.config.direct_countries.contains(&country_code) {
            debug!(
                "Country {} is in direct_countries list, using direct connection",
                country_code
            );
            return true;
        }

        // 6. 检查是否在代理国家列表中
        if !self.config.proxy_countries.is_empty()
            && self.config.proxy_countries.contains(&country_code)
        {
            debug!(
                "Country {} is in proxy_countries list, using proxy",
                country_code
            );
            return false;
        }

        // 7. 不在任何列表中，使用默认策略
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
            .field("direct_networks", &self.direct_networks)
            .field("proxy_networks", &self.proxy_networks)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_domain_matches() {
        // 通配符匹配
        assert!(GeoIpRouter::domain_matches(
            "www.google.com",
            "*.google.com"
        ));
        assert!(GeoIpRouter::domain_matches(
            "api.google.com",
            "*.google.com"
        ));
        assert!(GeoIpRouter::domain_matches("google.com", "*.google.com")); // 也匹配根域名
        assert!(!GeoIpRouter::domain_matches("google.cn", "*.google.com"));

        // 点前缀匹配
        assert!(GeoIpRouter::domain_matches(
            "www.example.com",
            ".example.com"
        ));
        assert!(!GeoIpRouter::domain_matches("example.com", ".example.com")); // 不匹配根域名

        // 精确匹配
        assert!(GeoIpRouter::domain_matches("example.com", "example.com"));
        assert!(!GeoIpRouter::domain_matches(
            "www.example.com",
            "example.com"
        ));
    }

    #[test]
    fn test_default_strategy() {
        let config = RoutingConfig {
            geoip_db: None,
            direct_countries: vec![],
            proxy_countries: vec![],
            direct_ips: vec![],
            proxy_ips: vec![],
            direct_domains: vec![],
            proxy_domains: vec![],
            default_strategy: RoutingStrategy::Direct,
        };

        let router = GeoIpRouter::new(config).unwrap();
        // 没有数据库时应该使用默认策略
        assert!(router.should_direct_connect("8.8.8.8:80"));
    }
}
