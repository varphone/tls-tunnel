/// Common utilities for integration tests
use std::net::TcpListener;
use std::path::PathBuf;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener as TokioTcpListener, TcpStream};
use tokio::time::timeout;

/// Find an available port
pub fn get_available_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("Failed to bind to random port")
        .local_addr()
        .expect("Failed to get local addr")
        .port()
}

/// Generate temporary certificate files for testing
pub fn generate_test_certs() -> (PathBuf, PathBuf) {
    use std::sync::atomic::{AtomicU64, Ordering};
    use tls_tunnel::tls;

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let temp_dir = std::env::temp_dir();
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64;
    let counter = COUNTER.fetch_add(1, Ordering::SeqCst);
    let unique_id = format!("{}-{}-{}", timestamp, counter, std::process::id());

    let cert_path = temp_dir.join(format!("test-cert-{}.pem", unique_id));
    let key_path = temp_dir.join(format!("test-key-{}.pem", unique_id));

    tls::generate_self_signed_cert(
        "localhost",
        &["127.0.0.1".to_string(), "localhost".to_string()],
        &cert_path,
        &key_path,
    )
    .expect("Failed to generate test certificates");

    (cert_path, key_path)
}

/// Create a simple echo server for testing
pub async fn start_echo_server(port: u16) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let listener = TokioTcpListener::bind(format!("127.0.0.1:{}", port))
            .await
            .expect("Failed to bind echo server");

        loop {
            match listener.accept().await {
                Ok((mut socket, _)) => {
                    tokio::spawn(async move {
                        let mut buf = vec![0u8; 8192];
                        loop {
                            match socket.read(&mut buf).await {
                                Ok(0) => break, // Connection closed
                                Ok(n) => {
                                    if socket.write_all(&buf[..n]).await.is_err() {
                                        break;
                                    }
                                }
                                Err(_) => break,
                            }
                        }
                    });
                }
                Err(_) => break,
            }
        }
    })
}

/// Test data transmission through proxy
pub async fn test_proxy_connection(
    proxy_port: u16,
    test_data: &[u8],
    timeout_duration: Duration,
) -> Result<Vec<u8>, String> {
    let result = timeout(timeout_duration, async {
        let mut stream = TcpStream::connect(format!("127.0.0.1:{}", proxy_port))
            .await
            .map_err(|e| format!("Failed to connect to proxy: {}", e))?;

        stream
            .write_all(test_data)
            .await
            .map_err(|e| format!("Failed to write data: {}", e))?;

        let mut response = Vec::new();
        let mut buf = vec![0u8; 8192];

        loop {
            match stream.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => {
                    response.extend_from_slice(&buf[..n]);
                    if response.len() >= test_data.len() {
                        break;
                    }
                }
                Err(e) => return Err(format!("Failed to read response: {}", e)),
            }
        }

        Ok(response)
    })
    .await
    .map_err(|_| "Timeout waiting for response".to_string())?;

    result
}

/// Wait for server to be ready
pub async fn wait_for_server(port: u16, max_attempts: u32) -> bool {
    for _ in 0..max_attempts {
        if TcpStream::connect(format!("127.0.0.1:{}", port))
            .await
            .is_ok()
        {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}

/// Cleanup function for test resources
pub struct TestCleanup {
    cert_path: Option<PathBuf>,
    key_path: Option<PathBuf>,
}

impl TestCleanup {
    pub fn new(cert_path: PathBuf, key_path: PathBuf) -> Self {
        Self {
            cert_path: Some(cert_path),
            key_path: Some(key_path),
        }
    }
}

impl Drop for TestCleanup {
    fn drop(&mut self) {
        if let Some(cert) = self.cert_path.take() {
            let _ = std::fs::remove_file(cert);
        }
        if let Some(key) = self.key_path.take() {
            let _ = std::fs::remove_file(key);
        }
    }
}
