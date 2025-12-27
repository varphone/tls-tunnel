/// Comprehensive integration tests for TLS Tunnel
mod common;

use std::time::Duration;
use tls_tunnel::config::{ClientConfig, ClientFullConfig, ProxyConfig, ServerConfig};
use tls_tunnel::transport::TransportType;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::sleep;
use tokio_rustls::{TlsAcceptor, TlsConnector};

// Helper function to create server config
fn create_server_config(
    port: u16,
    auth_key: &str,
    cert: &std::path::PathBuf,
    key: &std::path::PathBuf,
    transport: TransportType,
) -> ServerConfig {
    ServerConfig {
        bind_addr: "127.0.0.1".to_string(),
        bind_port: port,
        auth_key: auth_key.to_string(),
        cert_path: Some(cert.clone()),
        key_path: Some(key.clone()),
        transport,
        behind_proxy: false,
        allow_forward: false,
        rate_limit: None,
        size_limits: None,
        stats_port: None,
        stats_addr: None,
    }
}

// Helper function to create client config
fn create_client_config(
    server_port: u16,
    proxy_port: u16,
    echo_port: u16,
    auth_key: &str,
    cert: &std::path::PathBuf,
    transport: TransportType,
) -> ClientFullConfig {
    ClientFullConfig {
        client: ClientConfig {
            server_addr: "127.0.0.1".to_string(),
            server_port,
            server_path: "/".to_string(),
            auth_key: auth_key.to_string(),
            ca_cert_path: Some(cert.clone()),
            skip_verify: true,
            transport,
            stats_port: None,
            stats_addr: None,
        },
        proxies: vec![ProxyConfig {
            name: "test-proxy".to_string(),
            publish_addr: "127.0.0.1".to_string(),
            publish_port: proxy_port,
            local_port: echo_port,
            proxy_type: tls_tunnel::config::ProxyType::Tcp,
        }],
        visitors: vec![],
        forwarders: vec![],
    }
}

#[tokio::test]
async fn test_basic_tcp_proxy() {
    let server_port = common::get_available_port();
    let proxy_port = common::get_available_port();
    let echo_port = common::get_available_port();
    let auth_key = "test-basic";

    let (cert_path, key_path) = common::generate_test_certs();
    let _cleanup = common::TestCleanup::new(cert_path.clone(), key_path.clone());

    let _echo_server = common::start_echo_server(echo_port).await;
    sleep(Duration::from_millis(100)).await;

    // Start server
    let server_config = create_server_config(
        server_port,
        auth_key,
        &cert_path,
        &key_path,
        TransportType::Tls,
    );
    let tls_config = tls_tunnel::tls::load_server_config_with_alpn(&cert_path, &key_path, None)
        .expect("Failed to load server TLS config");
    let acceptor = TlsAcceptor::from(tls_config);

    let server_handle = tokio::spawn(async move {
        tls_tunnel::server::run_server(server_config, acceptor)
            .await
            .ok();
    });

    sleep(Duration::from_millis(300)).await;

    // Start client
    let client_config = create_client_config(
        server_port,
        proxy_port,
        echo_port,
        auth_key,
        &cert_path,
        TransportType::Tls,
    );
    let tls_config = tls_tunnel::tls::load_client_config_with_alpn(Some(&cert_path), true, None)
        .expect("Failed to load client TLS config");
    let connector = TlsConnector::from(tls_config);

    let client_handle = tokio::spawn(async move {
        tls_tunnel::client::run_client(client_config, connector)
            .await
            .ok();
    });

    sleep(Duration::from_millis(500)).await;

    // Test
    let test_data = b"Hello, World!";
    let response = common::test_proxy_connection(proxy_port, test_data, Duration::from_secs(5))
        .await
        .expect("Failed to test proxy connection");

    assert_eq!(response, test_data, "Response should match sent data");

    server_handle.abort();
    client_handle.abort();
}

#[tokio::test]
async fn test_auth_correct_key() {
    let server_port = common::get_available_port();
    let proxy_port = common::get_available_port();
    let echo_port = common::get_available_port();
    let auth_key = "correct-key-123";

    let (cert_path, key_path) = common::generate_test_certs();
    let _cleanup = common::TestCleanup::new(cert_path.clone(), key_path.clone());

    let _echo_server = common::start_echo_server(echo_port).await;
    sleep(Duration::from_millis(100)).await;

    let server_config = create_server_config(
        server_port,
        auth_key,
        &cert_path,
        &key_path,
        TransportType::Tls,
    );
    let tls_config = tls_tunnel::tls::load_server_config_with_alpn(&cert_path, &key_path, None)
        .expect("Failed to load server TLS config");
    let acceptor = TlsAcceptor::from(tls_config);

    let server_handle = tokio::spawn(async move {
        tls_tunnel::server::run_server(server_config, acceptor)
            .await
            .ok();
    });

    sleep(Duration::from_millis(300)).await;

    let client_config = create_client_config(
        server_port,
        proxy_port,
        echo_port,
        auth_key,
        &cert_path,
        TransportType::Tls,
    );
    let tls_config = tls_tunnel::tls::load_client_config_with_alpn(Some(&cert_path), true, None)
        .expect("Failed to load client TLS config");
    let connector = TlsConnector::from(tls_config);

    let client_handle = tokio::spawn(async move {
        tls_tunnel::client::run_client(client_config, connector)
            .await
            .ok();
    });

    sleep(Duration::from_millis(800)).await;

    let test_data = b"Auth test";
    let result = common::test_proxy_connection(proxy_port, test_data, Duration::from_secs(3)).await;

    assert!(result.is_ok(), "Should connect with correct auth key");
    assert_eq!(result.unwrap(), test_data);

    server_handle.abort();
    client_handle.abort();
}

#[tokio::test]
async fn test_auth_wrong_key() {
    let server_port = common::get_available_port();
    let proxy_port = common::get_available_port();
    let echo_port = common::get_available_port();

    let (cert_path, key_path) = common::generate_test_certs();
    let _cleanup = common::TestCleanup::new(cert_path.clone(), key_path.clone());

    let _echo_server = common::start_echo_server(echo_port).await;
    sleep(Duration::from_millis(100)).await;

    // Server with one key
    let server_config = create_server_config(
        server_port,
        "correct-key",
        &cert_path,
        &key_path,
        TransportType::Tls,
    );
    let tls_config = tls_tunnel::tls::load_server_config_with_alpn(&cert_path, &key_path, None)
        .expect("Failed to load server TLS config");
    let acceptor = TlsAcceptor::from(tls_config);

    let server_handle = tokio::spawn(async move {
        tls_tunnel::server::run_server(server_config, acceptor)
            .await
            .ok();
    });

    sleep(Duration::from_millis(300)).await;

    // Client with wrong key
    let client_config = create_client_config(
        server_port,
        proxy_port,
        echo_port,
        "wrong-key", // Wrong!
        &cert_path,
        TransportType::Tls,
    );
    let tls_config = tls_tunnel::tls::load_client_config_with_alpn(Some(&cert_path), true, None)
        .expect("Failed to load client TLS config");
    let connector = TlsConnector::from(tls_config);

    let _client_handle = tokio::spawn(async move {
        let _ = tls_tunnel::client::run_client(client_config, connector).await;
    });

    sleep(Duration::from_secs(2)).await;

    let test_data = b"Should not work";
    let result = common::test_proxy_connection(proxy_port, test_data, Duration::from_secs(2)).await;

    assert!(result.is_err(), "Should fail with wrong auth key");

    server_handle.abort();
}

#[tokio::test]
async fn test_multiple_connections() {
    let server_port = common::get_available_port();
    let proxy_port = common::get_available_port();
    let echo_port = common::get_available_port();
    let auth_key = "test-multi";

    let (cert_path, key_path) = common::generate_test_certs();
    let _cleanup = common::TestCleanup::new(cert_path.clone(), key_path.clone());

    let _echo_server = common::start_echo_server(echo_port).await;
    sleep(Duration::from_millis(100)).await;

    let server_config = create_server_config(
        server_port,
        auth_key,
        &cert_path,
        &key_path,
        TransportType::Tls,
    );
    let tls_config = tls_tunnel::tls::load_server_config_with_alpn(&cert_path, &key_path, None)
        .expect("Failed to load server TLS config");
    let acceptor = TlsAcceptor::from(tls_config);

    let server_handle = tokio::spawn(async move {
        tls_tunnel::server::run_server(server_config, acceptor)
            .await
            .ok();
    });

    sleep(Duration::from_millis(300)).await;

    let client_config = create_client_config(
        server_port,
        proxy_port,
        echo_port,
        auth_key,
        &cert_path,
        TransportType::Tls,
    );
    let tls_config = tls_tunnel::tls::load_client_config_with_alpn(Some(&cert_path), true, None)
        .expect("Failed to load client TLS config");
    let connector = TlsConnector::from(tls_config);

    let client_handle = tokio::spawn(async move {
        tls_tunnel::client::run_client(client_config, connector)
            .await
            .ok();
    });

    sleep(Duration::from_millis(500)).await;

    // Test 10 concurrent connections
    let mut handles = vec![];
    for i in 0..10 {
        let test_data = format!("Message {}", i);
        handles.push(tokio::spawn(async move {
            common::test_proxy_connection(proxy_port, test_data.as_bytes(), Duration::from_secs(5))
                .await
        }));
    }

    let mut success_count = 0;
    for (i, handle) in handles.into_iter().enumerate() {
        let result = handle.await.expect("Task panicked");
        if let Ok(response) = result {
            let expected = format!("Message {}", i);
            if response == expected.as_bytes() {
                success_count += 1;
            }
        }
    }

    assert!(
        success_count >= 8,
        "Most connections should succeed: {}/10",
        success_count
    );

    server_handle.abort();
    client_handle.abort();
}

#[tokio::test]
async fn test_large_data_transfer() {
    let server_port = common::get_available_port();
    let proxy_port = common::get_available_port();
    let echo_port = common::get_available_port();
    let auth_key = "test-large";

    let (cert_path, key_path) = common::generate_test_certs();
    let _cleanup = common::TestCleanup::new(cert_path.clone(), key_path.clone());

    let _echo_server = common::start_echo_server(echo_port).await;
    sleep(Duration::from_millis(100)).await;

    let server_config = create_server_config(
        server_port,
        auth_key,
        &cert_path,
        &key_path,
        TransportType::Tls,
    );
    let tls_config = tls_tunnel::tls::load_server_config_with_alpn(&cert_path, &key_path, None)
        .expect("Failed to load server TLS config");
    let acceptor = TlsAcceptor::from(tls_config);

    let server_handle = tokio::spawn(async move {
        tls_tunnel::server::run_server(server_config, acceptor)
            .await
            .ok();
    });

    sleep(Duration::from_millis(300)).await;

    let client_config = create_client_config(
        server_port,
        proxy_port,
        echo_port,
        auth_key,
        &cert_path,
        TransportType::Tls,
    );
    let tls_config = tls_tunnel::tls::load_client_config_with_alpn(Some(&cert_path), true, None)
        .expect("Failed to load client TLS config");
    let connector = TlsConnector::from(tls_config);

    let client_handle = tokio::spawn(async move {
        tls_tunnel::client::run_client(client_config, connector)
            .await
            .ok();
    });

    sleep(Duration::from_millis(500)).await;

    // Transfer 512KB
    let test_data: Vec<u8> = (0..512 * 1024).map(|i| (i % 256) as u8).collect();
    let response = common::test_proxy_connection(proxy_port, &test_data, Duration::from_secs(10))
        .await
        .expect("Failed to transfer large data");

    assert_eq!(response.len(), test_data.len());
    assert_eq!(response, test_data);

    server_handle.abort();
    client_handle.abort();
}

#[tokio::test]
async fn test_wss_transport() {
    let server_port = common::get_available_port();
    let proxy_port = common::get_available_port();
    let echo_port = common::get_available_port();
    let auth_key = "test-wss";

    let (cert_path, key_path) = common::generate_test_certs();
    let _cleanup = common::TestCleanup::new(cert_path.clone(), key_path.clone());

    let _echo_server = common::start_echo_server(echo_port).await;
    sleep(Duration::from_millis(100)).await;

    let server_config = create_server_config(
        server_port,
        auth_key,
        &cert_path,
        &key_path,
        TransportType::Wss,
    );
    let tls_config = tls_tunnel::tls::load_server_config_with_alpn(&cert_path, &key_path, None)
        .expect("Failed to load server TLS config");
    let acceptor = TlsAcceptor::from(tls_config);

    let server_handle = tokio::spawn(async move {
        tls_tunnel::server::run_server(server_config, acceptor)
            .await
            .ok();
    });

    sleep(Duration::from_millis(300)).await;

    let client_config = create_client_config(
        server_port,
        proxy_port,
        echo_port,
        auth_key,
        &cert_path,
        TransportType::Wss,
    );
    let tls_config = tls_tunnel::tls::load_client_config_with_alpn(Some(&cert_path), true, None)
        .expect("Failed to load client TLS config");
    let connector = TlsConnector::from(tls_config);

    let client_handle = tokio::spawn(async move {
        tls_tunnel::client::run_client(client_config, connector)
            .await
            .ok();
    });

    sleep(Duration::from_millis(500)).await;

    let test_data = b"WebSocket test";
    let response = common::test_proxy_connection(proxy_port, test_data, Duration::from_secs(5))
        .await
        .expect("WSS transport should work");

    assert_eq!(response, test_data);

    server_handle.abort();
    client_handle.abort();
}

#[tokio::test]
async fn test_http2_transport() {
    let server_port = common::get_available_port();
    let proxy_port = common::get_available_port();
    let echo_port = common::get_available_port();
    let auth_key = "test-http2";

    let (cert_path, key_path) = common::generate_test_certs();
    let _cleanup = common::TestCleanup::new(cert_path.clone(), key_path.clone());

    let _echo_server = common::start_echo_server(echo_port).await;
    sleep(Duration::from_millis(100)).await;

    let server_config = create_server_config(
        server_port,
        auth_key,
        &cert_path,
        &key_path,
        TransportType::Http2,
    );
    let alpn_protocols = Some(vec![b"h2".to_vec()]);
    let tls_config =
        tls_tunnel::tls::load_server_config_with_alpn(&cert_path, &key_path, alpn_protocols)
            .expect("Failed to load server TLS config");
    let acceptor = TlsAcceptor::from(tls_config);

    let server_handle = tokio::spawn(async move {
        tls_tunnel::server::run_server(server_config, acceptor)
            .await
            .ok();
    });

    sleep(Duration::from_millis(300)).await;

    let client_config = create_client_config(
        server_port,
        proxy_port,
        echo_port,
        auth_key,
        &cert_path,
        TransportType::Http2,
    );
    let alpn_protocols = Some(vec![b"h2".to_vec()]);
    let tls_config =
        tls_tunnel::tls::load_client_config_with_alpn(Some(&cert_path), true, alpn_protocols)
            .expect("Failed to load client TLS config");
    let connector = TlsConnector::from(tls_config);

    let client_handle = tokio::spawn(async move {
        tls_tunnel::client::run_client(client_config, connector)
            .await
            .ok();
    });

    sleep(Duration::from_millis(500)).await;

    let test_data = b"HTTP/2 test";
    let response = common::test_proxy_connection(proxy_port, test_data, Duration::from_secs(5))
        .await
        .expect("HTTP/2 transport should work");

    assert_eq!(response, test_data);

    server_handle.abort();
    client_handle.abort();
}

// Visitor 模式测试：客户端C通过服务器中转访问客户端B的服务
#[tokio::test]
async fn test_visitor_mode() {
    // 初始化日志
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_test_writer()
        .try_init();

    use tls_tunnel::config::{ProxyType, VisitorConfig};

    let server_port = common::get_available_port();
    let publish_port = common::get_available_port(); // 服务器发布的端口（不使用）
    let visitor_port = common::get_available_port(); // visitor 客户端本地监听端口
    let echo_port = common::get_available_port(); // Echo 服务器端口
    let auth_key = "test-visitor";

    let (cert_path, key_path) = common::generate_test_certs();
    let _cleanup = common::TestCleanup::new(cert_path.clone(), key_path.clone());

    // 启动 Echo 服务器（模拟客户端B的本地服务）
    let _echo_server = common::start_echo_server(echo_port).await;
    sleep(Duration::from_millis(100)).await;

    // 配置服务器 - 作为中转服务器
    let server_config = ServerConfig {
        bind_addr: "127.0.0.1".to_string(),
        bind_port: server_port,
        auth_key: auth_key.to_string(),
        cert_path: Some(cert_path.clone()),
        key_path: Some(key_path.clone()),
        transport: TransportType::Tls,
        behind_proxy: false,
        allow_forward: false,
        rate_limit: None,
        size_limits: None,
        stats_port: None,
        stats_addr: None,
    };
    let tls_config = tls_tunnel::tls::load_server_config_with_alpn(&cert_path, &key_path, None)
        .expect("Failed to load server TLS config");
    let acceptor = TlsAcceptor::from(tls_config);

    let server_handle = tokio::spawn(async move {
        tls_tunnel::server::run_server(server_config, acceptor)
            .await
            .ok();
    });

    sleep(Duration::from_millis(300)).await;

    // 客户端B（proxy端）- 发布服务到服务器
    let client_b_config = ClientFullConfig {
        client: ClientConfig {
            server_addr: "127.0.0.1".to_string(),
            server_port,
            server_path: "/".to_string(),
            auth_key: auth_key.to_string(),
            ca_cert_path: Some(cert_path.clone()),
            skip_verify: true,
            transport: TransportType::Tls,
            stats_port: None,
            stats_addr: None,
        },
        proxies: vec![ProxyConfig {
            name: "visitor-test".to_string(),
            publish_addr: "127.0.0.1".to_string(),
            publish_port,          // 让服务器注册这个 proxy
            local_port: echo_port, // 指向本地 echo 服务器
            proxy_type: ProxyType::Tcp,
        }],
        visitors: vec![],
        forwarders: vec![],
    };
    let tls_config_b = tls_tunnel::tls::load_client_config_with_alpn(Some(&cert_path), true, None)
        .expect("Failed to load client TLS config");
    let connector_b = TlsConnector::from(tls_config_b);

    let client_b_handle = tokio::spawn(async move {
        tls_tunnel::client::run_client(client_b_config, connector_b)
            .await
            .ok();
    });

    sleep(Duration::from_millis(500)).await;

    // 客户端C（visitor端）- 访问客户端B的服务
    let client_c_config = ClientFullConfig {
        client: ClientConfig {
            server_addr: "127.0.0.1".to_string(),
            server_port,
            server_path: "/".to_string(),
            auth_key: auth_key.to_string(),
            ca_cert_path: Some(cert_path.clone()),
            skip_verify: true,
            transport: TransportType::Tls,
            stats_port: None,
            stats_addr: None,
        },
        proxies: vec![],
        visitors: vec![VisitorConfig {
            name: "visitor-test".to_string(),
            proxy_type: ProxyType::Tcp,
            bind_addr: "127.0.0.1".to_string(),
            bind_port: visitor_port, // 客户端C本地监听
            publish_port,            // 匹配客户端B的 proxy
        }],
        forwarders: vec![],
    };
    let tls_config_c = tls_tunnel::tls::load_client_config_with_alpn(Some(&cert_path), true, None)
        .expect("Failed to load client TLS config");
    let connector_c = TlsConnector::from(tls_config_c);

    let client_c_handle = tokio::spawn(async move {
        tls_tunnel::client::run_client(client_c_config, connector_c)
            .await
            .ok();
    });

    sleep(Duration::from_millis(500)).await;

    // 测试：连接到客户端C的 visitor 端口，数据应该通过服务器中转到达客户端B的 echo 服务器
    println!("Testing visitor connection on port {}", visitor_port);
    let test_data = b"Visitor mode test";
    let response = common::test_proxy_connection(visitor_port, test_data, Duration::from_secs(10))
        .await
        .expect("Visitor mode should work");

    assert_eq!(response, test_data);

    server_handle.abort();
    client_b_handle.abort();
    client_c_handle.abort();
}

// HTTP Forwarder 测试
#[tokio::test]
async fn test_forwarder_http_proxy() {
    use tls_tunnel::config::{ForwarderConfig, ProxyType, RoutingConfig};

    let server_port = common::get_available_port();
    let forwarder_port = common::get_available_port(); // HTTP 代理端口
    let target_port = common::get_available_port(); // 目标 HTTP 服务器端口
    let auth_key = "test-forwarder-http";

    let (cert_path, key_path) = common::generate_test_certs();
    let _cleanup = common::TestCleanup::new(cert_path.clone(), key_path.clone());

    // 启动简单的 HTTP 服务器
    let http_server = tokio::spawn(async move {
        let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{}", target_port))
            .await
            .expect("Failed to bind HTTP server");

        loop {
            if let Ok((socket, _)) = listener.accept().await {
                tokio::spawn(async move {
                    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

                    let (reader, mut writer) = tokio::io::split(socket);
                    let mut reader = BufReader::new(reader);

                    // 读取请求头
                    let mut _request_line = String::new();
                    let _ = reader.read_line(&mut _request_line).await;

                    // 读取剩余请求头直到空行
                    loop {
                        let mut line = String::new();
                        if reader.read_line(&mut line).await.is_err()
                            || line == "\r\n"
                            || line == "\n"
                        {
                            break;
                        }
                    }

                    // 简单的 HTTP 响应
                    let response = "HTTP/1.1 200 OK\r\nContent-Length: 13\r\nConnection: close\r\n\r\nHello, World!";
                    let _ = writer.write_all(response.as_bytes()).await;
                    let _ = writer.flush().await;
                });
            }
        }
    });

    sleep(Duration::from_millis(100)).await;

    // 配置服务器 - 允许 forwarder
    let server_config = ServerConfig {
        bind_addr: "127.0.0.1".to_string(),
        bind_port: server_port,
        auth_key: auth_key.to_string(),
        cert_path: Some(cert_path.clone()),
        key_path: Some(key_path.clone()),
        transport: TransportType::Tls,
        behind_proxy: false,
        allow_forward: true, // 允许 forwarder
        rate_limit: None,
        size_limits: None,
        stats_port: None,
        stats_addr: None,
    };
    let tls_config = tls_tunnel::tls::load_server_config_with_alpn(&cert_path, &key_path, None)
        .expect("Failed to load server TLS config");
    let acceptor = TlsAcceptor::from(tls_config);

    let server_handle = tokio::spawn(async move {
        tls_tunnel::server::run_server(server_config, acceptor)
            .await
            .ok();
    });

    sleep(Duration::from_millis(300)).await;

    // 配置客户端 - forwarder 模式（HTTP 代理）
    // 使用路由配置使 127.0.0.1 直连（避免被服务器安全检查拒绝）
    let client_config = ClientFullConfig {
        client: ClientConfig {
            server_addr: "127.0.0.1".to_string(),
            server_port,
            server_path: "/".to_string(),
            auth_key: auth_key.to_string(),
            ca_cert_path: Some(cert_path.clone()),
            skip_verify: true,
            transport: TransportType::Tls,
            stats_port: None,
            stats_addr: None,
        },
        proxies: vec![],
        visitors: vec![],
        forwarders: vec![ForwarderConfig {
            name: "http-proxy".to_string(),
            proxy_type: ProxyType::HttpProxy,
            bind_addr: "127.0.0.1".to_string(),
            bind_port: forwarder_port,
            // 配置路由：127.0.0.1 直连
            routing: Some(RoutingConfig {
                geoip_db: None,
                direct_countries: vec![],
                proxy_countries: vec![],
                direct_ips: vec!["127.0.0.0/8".to_string()], // 本地回环地址直连
                proxy_ips: vec![],
                direct_domains: vec![],
                proxy_domains: vec![],
                default_strategy: tls_tunnel::config::RoutingStrategy::Direct, // 默认直连
            }),
        }],
    };
    let tls_config = tls_tunnel::tls::load_client_config_with_alpn(Some(&cert_path), true, None)
        .expect("Failed to load client TLS config");
    let connector = TlsConnector::from(tls_config);

    let client_handle = tokio::spawn(async move {
        tls_tunnel::client::run_client(client_config, connector)
            .await
            .ok();
    });

    sleep(Duration::from_millis(500)).await;

    // 测试 HTTP 代理（使用绝对 URL 模式）
    // 注意：不能测试 CONNECT 隧道模式连接本地地址，因为客户端和服务器都有安全检查阻止访问本地/私有地址
    let mut stream = TcpStream::connect(format!("127.0.0.1:{}", forwarder_port))
        .await
        .expect("Failed to connect to HTTP proxy");

    // 发送带绝对 URL 的 HTTP GET 请求（HTTP 代理标准格式）
    let http_request = format!(
        "GET http://127.0.0.1:{}/  HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n",
        target_port, target_port
    );
    stream
        .write_all(http_request.as_bytes())
        .await
        .expect("Failed to write HTTP request");
    stream.flush().await.expect("Failed to flush");

    // 读取 HTTP 响应
    let mut response = Vec::new();
    loop {
        let mut buf = vec![0u8; 1024];
        match tokio::time::timeout(Duration::from_secs(2), stream.read(&mut buf)).await {
            Ok(Ok(0)) => break, // 连接关闭
            Ok(Ok(n)) => {
                response.extend_from_slice(&buf[..n]);
                // 如果已经收到完整响应，就停止读取
                if String::from_utf8_lossy(&response).contains("Hello, World!") {
                    break;
                }
            }
            Ok(Err(_)) | Err(_) => break, // 错误或超时
        }
    }

    let response_str = String::from_utf8_lossy(&response);
    assert!(
        response_str.contains("Hello, World!"),
        "Should receive HTTP response. Got: {}",
        response_str
    );

    http_server.abort();
    server_handle.abort();
    client_handle.abort();
}

// SOCKS5 Forwarder 测试
#[tokio::test]
async fn test_forwarder_socks5_proxy() {
    use tls_tunnel::config::{ForwarderConfig, ProxyType, RoutingConfig};

    let server_port = common::get_available_port();
    let forwarder_port = common::get_available_port(); // SOCKS5 代理端口
    let echo_port = common::get_available_port(); // Echo 服务器端口
    let auth_key = "test-forwarder-socks5";

    let (cert_path, key_path) = common::generate_test_certs();
    let _cleanup = common::TestCleanup::new(cert_path.clone(), key_path.clone());

    // 启动 Echo 服务器
    let _echo_server = common::start_echo_server(echo_port).await;
    sleep(Duration::from_millis(100)).await;

    // 配置服务器 - 允许 forwarder
    let server_config = ServerConfig {
        bind_addr: "127.0.0.1".to_string(),
        bind_port: server_port,
        auth_key: auth_key.to_string(),
        cert_path: Some(cert_path.clone()),
        key_path: Some(key_path.clone()),
        transport: TransportType::Tls,
        behind_proxy: false,
        allow_forward: true, // 允许 forwarder
        rate_limit: None,
        size_limits: None,
        stats_port: None,
        stats_addr: None,
    };
    let tls_config = tls_tunnel::tls::load_server_config_with_alpn(&cert_path, &key_path, None)
        .expect("Failed to load server TLS config");
    let acceptor = TlsAcceptor::from(tls_config);

    let server_handle = tokio::spawn(async move {
        tls_tunnel::server::run_server(server_config, acceptor)
            .await
            .ok();
    });

    sleep(Duration::from_millis(300)).await;

    // 配置客户端 - forwarder 模式（SOCKS5 代理）
    // 使用路由配置使 127.0.0.1 直连（避免被服务器安全检查拒绝）
    let client_config = ClientFullConfig {
        client: ClientConfig {
            server_addr: "127.0.0.1".to_string(),
            server_port,
            server_path: "/".to_string(),
            auth_key: auth_key.to_string(),
            ca_cert_path: Some(cert_path.clone()),
            skip_verify: true,
            transport: TransportType::Tls,
            stats_port: None,
            stats_addr: None,
        },
        proxies: vec![],
        visitors: vec![],
        forwarders: vec![ForwarderConfig {
            name: "socks5-proxy".to_string(),
            proxy_type: ProxyType::Socks5Proxy,
            bind_addr: "127.0.0.1".to_string(),
            bind_port: forwarder_port,
            // 配置路由：127.0.0.1 直连
            routing: Some(RoutingConfig {
                geoip_db: None,
                direct_countries: vec![],
                proxy_countries: vec![],
                direct_ips: vec!["127.0.0.0/8".to_string()], // 本地回环地址直连
                proxy_ips: vec![],
                direct_domains: vec![],
                proxy_domains: vec![],
                default_strategy: tls_tunnel::config::RoutingStrategy::Direct, // 默认直连
            }),
        }],
    };
    let tls_config = tls_tunnel::tls::load_client_config_with_alpn(Some(&cert_path), true, None)
        .expect("Failed to load client TLS config");
    let connector = TlsConnector::from(tls_config);

    let client_handle = tokio::spawn(async move {
        tls_tunnel::client::run_client(client_config, connector)
            .await
            .ok();
    });

    sleep(Duration::from_millis(500)).await;

    // 测试 SOCKS5 连接
    let mut stream = TcpStream::connect(format!("127.0.0.1:{}", forwarder_port))
        .await
        .expect("Failed to connect to SOCKS5 proxy");

    // SOCKS5 握手 - 无认证
    stream
        .write_all(&[0x05, 0x01, 0x00])
        .await
        .expect("Failed to write SOCKS5 greeting");

    let mut buf = [0u8; 2];
    stream
        .read_exact(&mut buf)
        .await
        .expect("Failed to read SOCKS5 greeting response");
    assert_eq!(buf, [0x05, 0x00], "SOCKS5 greeting should succeed");

    // SOCKS5 CONNECT 请求 - IPv4
    let mut connect_request = vec![0x05, 0x01, 0x00, 0x01]; // VER CMD RSV ATYP
    connect_request.extend_from_slice(&[127, 0, 0, 1]); // 127.0.0.1
    connect_request.extend_from_slice(&echo_port.to_be_bytes()); // Port

    stream
        .write_all(&connect_request)
        .await
        .expect("Failed to write SOCKS5 CONNECT");

    // 读取 CONNECT 响应
    let mut buf = [0u8; 10];
    stream
        .read_exact(&mut buf)
        .await
        .expect("Failed to read SOCKS5 CONNECT response");

    assert_eq!(buf[0], 0x05, "SOCKS5 version should be 5");
    assert_eq!(
        buf[1], 0x00,
        "SOCKS5 CONNECT should succeed, got reply: {}",
        buf[1]
    );

    // 等待连接完全建立
    sleep(Duration::from_millis(100)).await;

    // 通过 SOCKS5 代理发送数据到 echo 服务器
    let test_data = b"SOCKS5 forwarder test";
    stream
        .write_all(test_data)
        .await
        .expect("Failed to write test data");
    stream.flush().await.expect("Failed to flush");

    // 读取 echo 响应（使用超时）
    let mut response = vec![0u8; test_data.len()];
    match tokio::time::timeout(Duration::from_secs(2), stream.read_exact(&mut response)).await {
        Ok(Ok(_)) => {
            assert_eq!(response, test_data, "Should receive echoed data");
        }
        Ok(Err(e)) => {
            panic!("Failed to read echo response: {}", e);
        }
        Err(_) => {
            panic!("Timeout reading echo response");
        }
    }

    server_handle.abort();
    client_handle.abort();
}
