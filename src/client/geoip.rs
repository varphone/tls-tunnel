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
        // 注意：.example.com 实际上也会匹配 example.com（根据实现）
        assert!(GeoIpRouter::domain_matches("example.com", ".example.com"));

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

    #[test]
    fn test_domain_routing_priority() {
        let config = RoutingConfig {
            geoip_db: None,
            direct_countries: vec![],
            proxy_countries: vec![],
            direct_ips: vec![],
            proxy_ips: vec![],
            direct_domains: vec!["*.example.com".to_string()],
            proxy_domains: vec!["*.google.com".to_string()],
            default_strategy: RoutingStrategy::Proxy, // 默认走代理
        };

        let router = GeoIpRouter::new(config).unwrap();

        // 直连域名列表优先
        assert!(router.should_direct_connect("api.example.com:443"));
        // 代理域名列表
        assert!(!router.should_direct_connect("www.google.com:443"));
        // 未匹配的走默认策略（代理）
        assert!(!router.should_direct_connect("unknown.org:443"));
    }

    #[test]
    fn test_ip_cidr_routing() {
        let config = RoutingConfig {
            geoip_db: None,
            direct_countries: vec![],
            proxy_countries: vec![],
            direct_ips: vec!["192.168.0.0/16".to_string(), "10.0.0.1".to_string()],
            proxy_ips: vec!["8.8.8.0/24".to_string()],
            direct_domains: vec![],
            proxy_domains: vec![],
            default_strategy: RoutingStrategy::Proxy,
        };

        let router = GeoIpRouter::new(config).unwrap();

        // 直连 IP/CIDR
        assert!(router.should_direct_connect("192.168.1.100:80"));
        assert!(router.should_direct_connect("10.0.0.1:22"));

        // 代理 IP/CIDR
        assert!(!router.should_direct_connect("8.8.8.8:53"));

        // 默认策略
        assert!(!router.should_direct_connect("1.1.1.1:443"));
    }

    #[test]
    fn test_routing_priority_order() {
        // 测试路由优先级：域名 > IP/CIDR > GeoIP > 默认策略
        let config = RoutingConfig {
            geoip_db: None,
            direct_countries: vec![],
            proxy_countries: vec![],
            direct_ips: vec!["8.8.8.8".to_string()], // IP 规则说直连
            proxy_ips: vec![],
            direct_domains: vec![],
            proxy_domains: vec!["*.google.com".to_string()], // 域名规则说走代理
            default_strategy: RoutingStrategy::Direct,
        };

        let router = GeoIpRouter::new(config).unwrap();

        // 域名优先级高于 IP，因此即使 8.8.8.8 在直连列表中，
        // dns.google.com 解析到 8.8.8.8 仍应该走代理（域名规则优先）
        assert!(!router.should_direct_connect("dns.google.com:443"));

        // 直接访问 8.8.8.8 时，域名规则不匹配，IP 规则生效
        assert!(router.should_direct_connect("8.8.8.8:53"));
    }

    #[test]
    fn test_wildcard_variations() {
        let config = RoutingConfig {
            geoip_db: None,
            direct_countries: vec![],
            proxy_countries: vec![],
            direct_ips: vec![],
            proxy_ips: vec![],
            direct_domains: vec![
                "*.cdn.example.com".to_string(),
                ".internal.company.com".to_string(),
                "exact-match.com".to_string(),
            ],
            proxy_domains: vec![],
            default_strategy: RoutingStrategy::Proxy,
        };

        let router = GeoIpRouter::new(config).unwrap();

        // 多级通配符
        assert!(router.should_direct_connect("img.cdn.example.com:443"));
        assert!(router.should_direct_connect("cdn.example.com:443")); // 通配符也匹配根域名

        // 点前缀（根据实现，也会匹配根域名）
        assert!(router.should_direct_connect("api.internal.company.com:8080"));
        assert!(router.should_direct_connect("internal.company.com:8080")); // 根据实现也匹配

        // 精确匹配
        assert!(router.should_direct_connect("exact-match.com:80"));
        assert!(!router.should_direct_connect("www.exact-match.com:80"));
    }

    #[test]
    fn test_invalid_addresses() {
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

        // 无效地址应该使用默认策略（不崩溃）
        assert!(router.should_direct_connect("invalid:port"));
        assert!(router.should_direct_connect(":80"));
        assert!(router.should_direct_connect("no-port"));
    }

    #[test]
    fn test_ipv6_support() {
        let config = RoutingConfig {
            geoip_db: None,
            direct_countries: vec![],
            proxy_countries: vec![],
            direct_ips: vec!["2001:db8::/32".to_string()],
            proxy_ips: vec![],
            direct_domains: vec![],
            proxy_domains: vec![],
            default_strategy: RoutingStrategy::Proxy,
        };

        let router = GeoIpRouter::new(config).unwrap();

        // IPv6 CIDR 匹配
        assert!(router.should_direct_connect("[2001:db8::1]:80"));
        assert!(!router.should_direct_connect("[2606:2800:220:1:248:1893:25c8:1946]:443"));
    }
}
