mod cli;
mod client;
mod config;
mod connection_pool;
mod server;
mod tls;
mod transport;

use anyhow::{Context, Result};
use clap::Parser;
use cli::{Cli, Commands};
use config::AppConfig;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio_rustls::{TlsAcceptor, TlsConnector};
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialize logging based on verbosity level
    let log_level = match cli.verbose {
        0 => "off",
        1 => "info",
        2 => "debug",
        _ => "trace",
    };

    tracing_subscriber::fmt()
        .with_env_filter(log_level)
        .with_target(false)
        .with_thread_ids(false)
        .with_file(false)
        .with_line_number(false)
        .init();

    // Display version information
    info!("TLS Tunnel v{}", env!("CARGO_PKG_VERSION"));

    match &cli.command {
        Commands::Check { config } => {
            check_config(config)?;
            return Ok(());
        }
        Commands::Template {
            template_type,
            output,
        } => {
            generate_config_template(template_type, output.as_deref())?;
            return Ok(());
        }
        Commands::Cert {
            cert_out,
            key_out,
            common_name,
            alt_names,
        } => {
            generate_certificate(cert_out, key_out, common_name, alt_names)?;
            return Ok(());
        }
        Commands::Register {
            service_type,
            config,
            name,
            exec,
        } => {
            register_systemd_service(service_type, config, name.as_deref(), exec.as_deref())?;
            return Ok(());
        }
        Commands::Unregister { service_type, name } => {
            unregister_systemd_service(service_type, name.as_deref())?;
            return Ok(());
        }
        Commands::Server { config } => {
            info!("Loading server configuration from: {}", config);
            let server_config = AppConfig::load_server_config(config)?;

            // 加载 TLS 配置
            let (cert_path, key_path) = ensure_server_certs(&server_config)?;

            // 根据传输类型设置 ALPN
            let alpn_protocols = if server_config.transport == transport::TransportType::Http2 {
                Some(vec![b"h2".to_vec()])
            } else {
                None
            };

            let tls_config =
                tls::load_server_config_with_alpn(&cert_path, &key_path, alpn_protocols)?;
            let acceptor = TlsAcceptor::from(tls_config);

            // 运行服务器
            server::run_server(server_config, acceptor).await?;
        }
        Commands::Client { config } => {
            info!("Loading client configuration from: {}", config);
            let client_config = AppConfig::load_client_config(config)?;

            // 加载 TLS 配置
            // 根据传输类型设置 ALPN
            let alpn_protocols =
                if client_config.client.transport == transport::TransportType::Http2 {
                    Some(vec![b"h2".to_vec()])
                } else {
                    None
                };

            let tls_config = tls::load_client_config_with_alpn(
                client_config.client.ca_cert_path.as_deref(),
                client_config.client.skip_verify,
                alpn_protocols,
            )?;
            let connector = TlsConnector::from(tls_config);

            // 运行客户端
            client::run_client(client_config, connector).await?;
        }
    }

    Ok(())
}

/// Generate configuration template
fn generate_config_template(template_type: &str, output: Option<&str>) -> Result<()> {
    let content = match template_type {
        "server" => include_str!("../examples/server-template.toml"),
        "client" => include_str!("../examples/client-template.toml"),
        _ => unreachable!(),
    };

    if let Some(path) = output {
        std::fs::write(path, content)
            .with_context(|| format!("Failed to write config template to {}", path))?;
        println!("Generated {} configuration template: {}", template_type, path);
    } else {
        println!("{}", content);
    }

    Ok(())
}

/// Generate self-signed TLS certificate
fn generate_certificate(
    cert_out: &str,
    key_out: &str,
    common_name: &str,
    alt_names: &[String],
) -> Result<()> {
    let mut sans = if alt_names.is_empty() {
        vec![common_name.to_string()]
    } else {
        alt_names.to_vec()
    };

    if !sans.iter().any(|n| n == common_name) {
        sans.push(common_name.to_string());
    }

    tls::generate_self_signed_cert(
        common_name,
        &sans,
        Path::new(cert_out),
        Path::new(key_out),
    )?;

    println!("Generated self-signed certificate: {}", cert_out);
    println!("Generated private key: {}", key_out);

    Ok(())
}

/// Register as systemd service (Linux only)
fn register_systemd_service(
    _service_type: &str,
    _config: &str,
    _name: Option<&str>,
    _exec: Option<&str>,
) -> Result<()> {
    #[cfg(not(target_os = "linux"))]
    {
        anyhow::bail!("Service registration is only supported on Linux");
    }

    #[cfg(target_os = "linux")]
    {
        use std::process::Command;

        let service_name = _name.unwrap_or_else(|| match _service_type {
            "server" => "tls-tunnel-server",
            "client" => "tls-tunnel-client",
            _ => unreachable!(),
        });

        let exec_path = if let Some(custom) = _exec {
            custom.to_string()
        } else {
            std::env::current_exe()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| "tls-tunnel".to_string())
        };

        let config_path = std::fs::canonicalize(_config)
            .with_context(|| format!("Failed to resolve config path: {}", config))?
            .to_string_lossy()
            .into_owned();

        let description = match _service_type {
            "server" => "TLS Tunnel Server",
            "client" => "TLS Tunnel Client",
            _ => unreachable!(),
        };

        let unit_content = format!(
            "[Unit]\n\
            Description={}\n\
            After=network-online.target\n\
            Wants=network-online.target\n\
            \n\
            [Service]\n\
            Type=simple\n\
            ExecStart={} {} --config {}\n\
            Restart=on-failure\n\
            RestartSec=3\n\
            Environment=RUST_LOG=info\n\
            \n\
            [Install]\n\
            WantedBy=multi-user.target\n",
            description, exec_path, _service_type, config_path
        );

        let unit_file = format!("/etc/systemd/system/{}.service", service_name);

        // Write systemd unit file
        std::fs::write(&unit_file, unit_content)
            .with_context(|| format!("Failed to write systemd unit file to {}", unit_file))?;

        println!("✓ Created systemd service file: {}", unit_file);

        // Reload systemd daemon
        let output = Command::new("systemctl")
            .arg("daemon-reload")
            .output()
            .context("Failed to reload systemd daemon")?;

        if !output.status.success() {
            anyhow::bail!(
                "Failed to reload systemd daemon: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        println!("✓ Reloaded systemd daemon");

        // Enable service
        let output = Command::new("systemctl")
            .arg("enable")
            .arg(format!("{}.service", service_name))
            .output()
            .context("Failed to enable service")?;

        if !output.status.success() {
            anyhow::bail!(
                "Failed to enable service: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        println!("✓ Enabled {} service", service_name);
        println!("\nService registered successfully!");
        println!("Start service: sudo systemctl start {}", service_name);
        println!("Check status:  sudo systemctl status {}", service_name);
        println!("View logs:     sudo journalctl -u {} -f", service_name);

        Ok(())
    }
}

/// Unregister systemd service (Linux only)
fn unregister_systemd_service(_service_type: &str, _name: Option<&str>) -> Result<()> {
    #[cfg(not(target_os = "linux"))]
    {
        anyhow::bail!("Service unregistration is only supported on Linux");
    }

    #[cfg(target_os = "linux")]
    {
        use std::process::Command;

        let service_name = _name.unwrap_or_else(|| match _service_type {
            "server" => "tls-tunnel-server",
            "client" => "tls-tunnel-client",
            _ => unreachable!(),
        });

        let unit_file = format!("/etc/systemd/system/{}.service", service_name);

        // Check if service exists
        if !Path::new(&unit_file).exists() {
            anyhow::bail!("Service {} not found", service_name);
        }

        // Stop service if running
        let output = Command::new("systemctl")
            .arg("stop")
            .arg(format!("{}.service", service_name))
            .output()
            .context("Failed to stop service")?;

        if output.status.success() {
            println!("✓ Stopped {} service", service_name);
        }

        // Disable service
        let output = Command::new("systemctl")
            .arg("disable")
            .arg(format!("{}.service", service_name))
            .output()
            .context("Failed to disable service")?;

        if output.status.success() {
            println!("✓ Disabled {} service", service_name);
        }

        // Remove service file
        std::fs::remove_file(&unit_file)
            .with_context(|| format!("Failed to remove service file: {}", unit_file))?;

        println!("✓ Removed service file: {}", unit_file);

        // Reload systemd daemon
        let output = Command::new("systemctl")
            .arg("daemon-reload")
            .output()
            .context("Failed to reload systemd daemon")?;

        if !output.status.success() {
            anyhow::bail!(
                "Failed to reload systemd daemon: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        println!("✓ Reloaded systemd daemon");
        println!("\nService unregistered successfully!");

        Ok(())
    }
}

/// Ensure server TLS certificates are available; generate self-signed certificates at runtime if not configured
fn ensure_server_certs(config: &config::ServerConfig) -> Result<(PathBuf, PathBuf)> {
    match (&config.cert_path, &config.key_path) {
        (Some(cert), Some(key)) => Ok((cert.clone(), key.clone())),
        (None, None) => {
            let ts = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let temp_dir = std::env::temp_dir();
            let cert_path = temp_dir.join(format!("tls-tunnel-cert-{}.pem", ts));
            let key_path = temp_dir.join(format!("tls-tunnel-key-{}.pem", ts));

            // 使用绑定地址作为 CN/SAN（若为 0.0.0.0 则回退 localhost）
            let cn = if config.bind_addr == "0.0.0.0" {
                "localhost"
            } else {
                config.bind_addr.as_str()
            };
            let alt = vec![cn.to_string()];

            tls::generate_self_signed_cert(cn, &alt, &cert_path, &key_path)?;

            info!(
                "Generated self-signed server certificate at {:?} and key at {:?}",
                cert_path, key_path
            );

            Ok((cert_path, key_path))
        }
        _ => anyhow::bail!(
            "Both cert_path and key_path must be set, or leave both empty to auto-generate"
        ),
    }
}

/// Check configuration file format
fn check_config(config_path: &str) -> Result<()> {
    use std::path::Path;

    let path = Path::new(config_path);

    // 检查文件是否存在
    if !path.exists() {
        anyhow::bail!("Configuration file not found: {}", config_path);
    }

    println!("Checking configuration file: {}\n", config_path);

    // 尝试作为服务器配置加载
    if let Ok(server_config) = AppConfig::load_server_config(config_path) {
        println!("✓ Configuration type: Server");
        println!("✓ Bind address: {}", server_config.bind_addr);
        println!("✓ Bind port: {}", server_config.bind_port);
        println!("✓ Auth key: {} characters", server_config.auth_key.len());
        match (&server_config.cert_path, &server_config.key_path) {
            (Some(cert), Some(key)) => {
                println!("✓ Certificate path: {:?}", cert);
                println!("✓ Key path: {:?}", key);

                if !cert.exists() {
                    println!("⚠ Warning: Certificate file not found: {:?}", cert);
                } else {
                    println!("✓ Certificate file exists");
                }

                if !key.exists() {
                    println!("⚠ Warning: Key file not found: {:?}", key);
                } else {
                    println!("✓ Key file exists");
                }
            }
            (None, None) => {
                println!("✓ Certificate/Key: will be auto-generated at runtime");
            }
            _ => {
                println!(
                    "✗ cert_path/key_path mismatch: set both, or leave both empty to auto-generate"
                );
                anyhow::bail!(
                    "cert_path and key_path must both be set, or both omitted to auto-generate"
                );
            }
        }

        println!("\n✓ Server configuration is valid!");
        return Ok(());
    }

    // 尝试作为客户端配置加载
    match AppConfig::load_client_config(config_path) {
        Ok(client_config) => {
            println!("✓ Configuration type: Client");
            println!("✓ Server address: {}", client_config.client.server_addr);
            println!("✓ Server port: {}", client_config.client.server_port);
            println!("✓ Skip verify: {}", client_config.client.skip_verify);
            println!(
                "✓ Auth key: {} characters",
                client_config.client.auth_key.len()
            );

            if let Some(ref ca_path) = client_config.client.ca_cert_path {
                println!("✓ CA certificate path: {:?}", ca_path);
                if !ca_path.exists() {
                    println!("⚠ Warning: CA certificate file not found: {:?}", ca_path);
                } else {
                    println!("✓ CA certificate file exists");
                }
            }

            println!("✓ Number of proxies: {}", client_config.proxies.len());

            if client_config.proxies.is_empty() {
                println!("⚠ Warning: No proxy configurations defined");
            } else {
                for (idx, proxy) in client_config.proxies.iter().enumerate() {
                    println!(
                        "  Proxy #{}: '{}' (publish_port={}, local_port={})",
                        idx + 1,
                        proxy.name,
                        proxy.publish_port,
                        proxy.local_port
                    );
                }
            }

            println!("\n✓ Client configuration is valid!");
            Ok(())
        }
        Err(e) => {
            println!("✗ Configuration validation failed!");
            println!("\nError details:");
            println!("{:#}", e);

            // 提供一些常见问题的提示
            println!("\nCommon issues:");
            println!("  1. Check TOML syntax (brackets, quotes, commas)");
            println!("  2. Ensure all required fields are present");
            println!("  3. Verify field names are spelled correctly");
            println!("  4. Check that paths use forward slashes or escaped backslashes");
            println!("  5. Ensure port numbers are valid (1-65535)");
            println!("  6. For server config: [server] section with bind_addr, bind_port, auth_key, and optional cert_path/key_path (omit both to auto-generate)");
            println!("  7. For client config: [client] section with server_addr, server_port, auth_key, and [[proxies]] sections");

            Err(e)
        }
    }
}

