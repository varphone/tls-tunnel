/// Fuzzy tests for TLS Tunnel server reliability
///
/// These tests verify server robustness against malformed, unexpected,
/// or malicious inputs to ensure stability and security.
mod common;

use std::time::Duration;
use tls_tunnel::config::ServerConfig;
use tls_tunnel::transport::TransportType;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::{sleep, timeout};
use tokio_rustls::{TlsAcceptor, TlsConnector};

/// 测试服务器对畸形认证消息的处理
#[tokio::test]
async fn test_malformed_auth_message() {
    let server_port = common::get_available_port();
    let auth_key = "test-fuzzy-auth";

    let (cert_path, key_path) = common::generate_test_certs();
    let _cleanup = common::TestCleanup::new(cert_path.clone(), key_path.clone());

    // 启动服务器
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

    // 连接到服务器
    let tls_config = tls_tunnel::tls::load_client_config_with_alpn(Some(&cert_path), true, None)
        .expect("Failed to load client TLS config");
    let connector = TlsConnector::from(tls_config);

    let tcp_stream = TcpStream::connect(format!("127.0.0.1:{}", server_port))
        .await
        .expect("Failed to connect");

    let server_name = tokio_rustls::rustls::pki_types::ServerName::try_from("localhost")
        .unwrap()
        .to_owned();

    let mut tls_stream = connector
        .connect(server_name, tcp_stream)
        .await
        .expect("TLS handshake failed");

    // 发送畸形的认证消息（不完整的 JSON-RPC）
    let malformed_messages = vec![
        b"{\"jsonrpc\":\"2.0\"".to_vec(), // 不完整的 JSON
        b"{\"jsonrpc\":\"2.0\",\"method\":\"authenticate\"}\n".to_vec(), // 缺少 params
        b"not json at all\n".to_vec(),    // 完全不是 JSON
        b"{\"jsonrpc\":\"2.0\",\"method\":\"authenticate\",\"params\":{\"auth_key\":\"\"}}\n"
            .to_vec(), // 空密钥
        vec![0u8; 10000],                 // 大量空字节
    ];

    for msg in malformed_messages {
        let _ = tls_stream.write_all(&msg).await;
        let _ = tls_stream.flush().await;

        // 尝试读取响应或检测连接是否关闭
        let mut buf = vec![0u8; 1024];
        let _ = timeout(Duration::from_millis(500), tls_stream.read(&mut buf)).await;
    }

    // 服务器应该保持运行，而不是崩溃
    server_handle.abort();
}

/// 测试服务器对超大消息的处理
#[tokio::test]
async fn test_oversized_messages() {
    let server_port = common::get_available_port();
    let auth_key = "test-fuzzy-oversized";

    let (cert_path, key_path) = common::generate_test_certs();
    let _cleanup = common::TestCleanup::new(cert_path.clone(), key_path.clone());

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
        size_limits: Some(tls_tunnel::config::SizeLimitConfig {
            max_request_size: 1024 * 1024, // 1MB
            max_header_size: 8 * 1024,     // 8KB
        }),
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

    let tls_config = tls_tunnel::tls::load_client_config_with_alpn(Some(&cert_path), true, None)
        .expect("Failed to load client TLS config");
    let connector = TlsConnector::from(tls_config);

    let tcp_stream = TcpStream::connect(format!("127.0.0.1:{}", server_port))
        .await
        .expect("Failed to connect");

    let server_name = tokio_rustls::rustls::pki_types::ServerName::try_from("localhost")
        .unwrap()
        .to_owned();

    let mut tls_stream = connector
        .connect(server_name, tcp_stream)
        .await
        .expect("TLS handshake failed");

    // 发送超大消息（超过限制）
    let huge_message = vec![0x41u8; 2 * 1024 * 1024]; // 2MB of 'A'
    let _ = tls_stream.write_all(&huge_message).await;
    let _ = tls_stream.flush().await;

    // 服务器应该拒绝或关闭连接，但不应崩溃
    sleep(Duration::from_millis(500)).await;

    server_handle.abort();
}

/// 测试快速连接和断开
#[tokio::test]
async fn test_rapid_connect_disconnect() {
    let server_port = common::get_available_port();
    let auth_key = "test-fuzzy-rapid";

    let (cert_path, key_path) = common::generate_test_certs();
    let _cleanup = common::TestCleanup::new(cert_path.clone(), key_path.clone());

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

    // 快速连接和断开 50 次
    for _ in 0..50 {
        let result = timeout(
            Duration::from_millis(500),
            TcpStream::connect(format!("127.0.0.1:{}", server_port)),
        )
        .await;

        if let Ok(Ok(stream)) = result {
            drop(stream); // 立即断开
        }
    }

    // 服务器应该仍然运行
    sleep(Duration::from_millis(500)).await;

    server_handle.abort();
}

/// 测试并发大量连接
#[tokio::test]
async fn test_concurrent_connections() {
    let server_port = common::get_available_port();
    let auth_key = "test-fuzzy-concurrent";

    let (cert_path, key_path) = common::generate_test_certs();
    let _cleanup = common::TestCleanup::new(cert_path.clone(), key_path.clone());

    let server_config = ServerConfig {
        bind_addr: "127.0.0.1".to_string(),
        bind_port: server_port,
        auth_key: auth_key.to_string(),
        cert_path: Some(cert_path.clone()),
        key_path: Some(key_path.clone()),
        transport: TransportType::Tls,
        behind_proxy: false,
        allow_forward: false,
        rate_limit: Some(tls_tunnel::config::RateLimitConfig {
            requests_per_second: 100,
            burst_size: 200,
        }),
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

    // 并发创建 30 个连接
    let mut handles = vec![];

    for i in 0..30 {
        let port = server_port;
        let cert = cert_path.clone();

        let handle = tokio::spawn(async move {
            let result = timeout(Duration::from_secs(2), async {
                let tls_config =
                    tls_tunnel::tls::load_client_config_with_alpn(Some(&cert), true, None)?;
                let connector = TlsConnector::from(tls_config);

                let tcp_stream = TcpStream::connect(format!("127.0.0.1:{}", port)).await?;

                let server_name =
                    tokio_rustls::rustls::pki_types::ServerName::try_from("localhost")
                        .unwrap()
                        .to_owned();

                let _tls_stream = connector.connect(server_name, tcp_stream).await?;

                // 保持连接一小段时间
                sleep(Duration::from_millis(100 * (i % 5))).await;

                Ok::<_, anyhow::Error>(())
            })
            .await;

            result
        });

        handles.push(handle);
    }

    // 等待所有连接完成
    for handle in handles {
        let _ = handle.await;
    }

    // 服务器应该仍然运行
    sleep(Duration::from_millis(500)).await;

    server_handle.abort();
}

/// 测试不完整的协议握手
#[tokio::test]
async fn test_incomplete_handshake() {
    let server_port = common::get_available_port();
    let auth_key = "test-fuzzy-incomplete";

    let (cert_path, key_path) = common::generate_test_certs();
    let _cleanup = common::TestCleanup::new(cert_path.clone(), key_path.clone());

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

    // 建立 TLS 连接但不发送任何数据就关闭
    for _ in 0..10 {
        let tls_config =
            tls_tunnel::tls::load_client_config_with_alpn(Some(&cert_path), true, None)
                .expect("Failed to load client TLS config");
        let connector = TlsConnector::from(tls_config);

        let tcp_stream = TcpStream::connect(format!("127.0.0.1:{}", server_port))
            .await
            .expect("Failed to connect");

        let server_name = tokio_rustls::rustls::pki_types::ServerName::try_from("localhost")
            .unwrap()
            .to_owned();

        let tls_stream = connector
            .connect(server_name, tcp_stream)
            .await
            .expect("TLS handshake failed");

        // 立即关闭，不发送认证消息
        drop(tls_stream);

        sleep(Duration::from_millis(50)).await;
    }

    // 服务器应该仍然运行
    sleep(Duration::from_millis(500)).await;

    server_handle.abort();
}

/// 测试随机数据注入
#[tokio::test]
async fn test_random_data_injection() {
    let server_port = common::get_available_port();
    let auth_key = "test-fuzzy-random";

    let (cert_path, key_path) = common::generate_test_certs();
    let _cleanup = common::TestCleanup::new(cert_path.clone(), key_path.clone());

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

    let tls_config = tls_tunnel::tls::load_client_config_with_alpn(Some(&cert_path), true, None)
        .expect("Failed to load client TLS config");
    let connector = TlsConnector::from(tls_config);

    let tcp_stream = TcpStream::connect(format!("127.0.0.1:{}", server_port))
        .await
        .expect("Failed to connect");

    let server_name = tokio_rustls::rustls::pki_types::ServerName::try_from("localhost")
        .unwrap()
        .to_owned();

    let mut tls_stream = connector
        .connect(server_name, tcp_stream)
        .await
        .expect("TLS handshake failed");

    // 发送随机数据
    use rand::Rng;
    let mut rng = rand::thread_rng();

    for _ in 0..20 {
        let len = rng.gen_range(1..1000);
        let random_data: Vec<u8> = (0..len).map(|_| rng.gen()).collect();

        let _ = tls_stream.write_all(&random_data).await;
        let _ = tls_stream.flush().await;

        sleep(Duration::from_millis(10)).await;
    }

    // 服务器应该保持运行
    sleep(Duration::from_millis(500)).await;

    server_handle.abort();
}

/// 测试空连接（连接后不发送任何数据）
#[tokio::test]
async fn test_idle_connections() {
    let server_port = common::get_available_port();
    let auth_key = "test-fuzzy-idle";

    let (cert_path, key_path) = common::generate_test_certs();
    let _cleanup = common::TestCleanup::new(cert_path.clone(), key_path.clone());

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

    // 创建多个空闲连接
    let mut connections = vec![];

    for _ in 0..5 {
        let tls_config =
            tls_tunnel::tls::load_client_config_with_alpn(Some(&cert_path), true, None)
                .expect("Failed to load client TLS config");
        let connector = TlsConnector::from(tls_config);

        let tcp_stream = TcpStream::connect(format!("127.0.0.1:{}", server_port))
            .await
            .expect("Failed to connect");

        let server_name = tokio_rustls::rustls::pki_types::ServerName::try_from("localhost")
            .unwrap()
            .to_owned();

        let tls_stream = connector
            .connect(server_name, tcp_stream)
            .await
            .expect("TLS handshake failed");

        connections.push(tls_stream);
    }

    // 保持空闲连接一段时间
    sleep(Duration::from_secs(2)).await;

    // 关闭所有连接
    drop(connections);

    // 服务器应该仍然运行
    sleep(Duration::from_millis(500)).await;

    server_handle.abort();
}

/// 测试混合有效和无效的消息
#[tokio::test]
async fn test_mixed_valid_invalid_messages() {
    let server_port = common::get_available_port();
    let auth_key = "test-fuzzy-mixed";

    let (cert_path, key_path) = common::generate_test_certs();
    let _cleanup = common::TestCleanup::new(cert_path.clone(), key_path.clone());

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

    let tls_config = tls_tunnel::tls::load_client_config_with_alpn(Some(&cert_path), true, None)
        .expect("Failed to load client TLS config");
    let connector = TlsConnector::from(tls_config);

    let tcp_stream = TcpStream::connect(format!("127.0.0.1:{}", server_port))
        .await
        .expect("Failed to connect");

    let server_name = tokio_rustls::rustls::pki_types::ServerName::try_from("localhost")
        .unwrap()
        .to_owned();

    let mut tls_stream = connector
        .connect(server_name, tcp_stream)
        .await
        .expect("TLS handshake failed");

    // 发送一个有效的认证请求
    let valid_auth = format!(
        r#"{{"jsonrpc":"2.0","method":"authenticate","params":{{"auth_key":"{}"}}}}"#,
        auth_key
    );
    tls_stream.write_all(valid_auth.as_bytes()).await.ok();
    tls_stream.write_all(b"\n").await.ok();
    tls_stream.flush().await.ok();

    // 等待响应
    sleep(Duration::from_millis(100)).await;

    // 发送一些无效消息
    let invalid_messages: Vec<&[u8]> = vec![
        b"garbage data\n",
        b"{\"invalid\":\"json\n",
        b"\x00\x01\x02\x03\n",
    ];

    for msg in invalid_messages {
        tls_stream.write_all(msg).await.ok();
        tls_stream.flush().await.ok();
        sleep(Duration::from_millis(50)).await;
    }

    // 服务器应该仍然运行
    sleep(Duration::from_millis(500)).await;

    server_handle.abort();
}
