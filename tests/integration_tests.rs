/// Comprehensive integration tests for TLS Tunnel
mod common;

use std::time::Duration;
use tls_tunnel::config::{ClientConfig, ClientFullConfig, ProxyConfig, ServerConfig};
use tls_tunnel::transport::TransportType;
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
