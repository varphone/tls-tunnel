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

struct GenerateOptions<'a> {
    config_type: &'a str,
    output: Option<&'a str>,
    cert_out: Option<&'a str>,
    key_out: Option<&'a str>,
    common_name: &'a str,
    alt_names: &'a [String],
    systemd_out: Option<&'a str>,
    service_config: Option<&'a str>,
    service_exec: Option<&'a str>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // 初始化日志
    tracing_subscriber::fmt()
        .with_env_filter(&cli.log_level)
        .with_target(false)
        .with_thread_ids(false)
        .with_file(false)
        .with_line_number(false)
        .init();

    // 显示版本信息
    info!("TLS Tunnel v{}", env!("CARGO_PKG_VERSION"));

    match &cli.command {
        Commands::Check { config } => {
            check_config(config)?;
            return Ok(());
        }
        Commands::Generate {
            config_type,
            output,
            cert_out,
            key_out,
            common_name,
            alt_names,
            systemd_out,
            service_config,
            service_exec,
        } => {
            handle_generate(GenerateOptions {
                config_type,
                output: output.as_deref(),
                cert_out: cert_out.as_deref(),
                key_out: key_out.as_deref(),
                common_name,
                alt_names,
                systemd_out: systemd_out.as_deref(),
                service_config: service_config.as_deref(),
                service_exec: service_exec.as_deref(),
            })?;
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

/// 处理生成示例配置、证书和 systemd 服务文件
fn handle_generate(opts: GenerateOptions<'_>) -> Result<()> {
    match opts.config_type {
        "server" | "client" => {
            generate_example_config(opts.config_type, opts.output)?;
        }
        "cert" | "systemd" => {
            // skip config generation
        }
        other => {
            anyhow::bail!("Unsupported generate type: {}", other);
        }
    }

    // 是否生成证书
    let should_gen_cert = match opts.config_type {
        "cert" => true,
        _ => opts.cert_out.is_some() || opts.key_out.is_some() || !opts.alt_names.is_empty(),
    };

    if should_gen_cert {
        let cert_path = opts.cert_out.unwrap_or("cert.pem");
        let key_path = opts.key_out.unwrap_or("key.pem");

        let mut sans = if opts.alt_names.is_empty() {
            vec![opts.common_name.to_string()]
        } else {
            opts.alt_names.to_vec()
        };

        if !sans.iter().any(|n| n == opts.common_name) {
            sans.push(opts.common_name.to_string());
        }

        tls::generate_self_signed_cert(
            opts.common_name,
            &sans,
            Path::new(cert_path),
            Path::new(key_path),
        )?;

        println!(
            "Generated self-signed certificate: {}\nGenerated private key: {}",
            cert_path, key_path
        );
    }

    let should_gen_systemd = match (opts.config_type, opts.systemd_out) {
        ("systemd", Some(_)) => true,
        ("systemd", None) => anyhow::bail!("--systemd-out is required when config_type=systemd"),
        (_, Some(_)) => true,
        _ => false,
    };

    if should_gen_systemd {
        let unit_out = opts.systemd_out.expect("unit path checked above");
        generate_systemd_unit(
            opts.config_type,
            Path::new(unit_out),
            opts.service_exec,
            opts.service_config,
        )?;

        println!("Generated systemd service file: {}", unit_out);
    }

    Ok(())
}

/// 确保服务器 TLS 证书与私钥可用；若未配置则在运行时生成自签名证书
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

/// 生成 systemd 服务文件
fn generate_systemd_unit(
    mode: &str,
    output: &Path,
    exec_path: Option<&str>,
    config_path: Option<&str>,
) -> Result<()> {
    let exec = if let Some(custom) = exec_path {
        custom.to_string()
    } else {
        std::env::current_exe()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| "tls-tunnel".to_string())
    };

    let config = if let Some(custom) = config_path {
        custom.to_string()
    } else if mode == "server" {
        "/etc/tls-tunnel/server.toml".to_string()
    } else {
        "/etc/tls-tunnel/client.toml".to_string()
    };

    let description = if mode == "server" {
        "TLS Tunnel Server"
    } else {
        "TLS Tunnel Client"
    };

    let unit = format!(
        "[Unit]\nDescription={}\nAfter=network-online.target\nWants=network-online.target\n\n[Service]\nType=simple\nExecStart={} {} --config {}\nRestart=on-failure\nRestartSec=3\nEnvironment=RUST_LOG=info\n\n[Install]\nWantedBy=multi-user.target\n",
        description, exec, mode, config
    );

    std::fs::write(output, unit)
        .with_context(|| format!("Failed to write systemd unit to {:?}", output))?;

    Ok(())
}

/// 检查配置文件格式
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

/// 生成示例配置文件
fn generate_example_config(config_type: &str, output: Option<&str>) -> Result<()> {
    let content = match config_type {
        "server" => {
            r#"[server]
# 服务器绑定地址
bind_addr = "0.0.0.0"
# 服务器监听端口
bind_port = 8443

# TLS 证书路径
cert_path = "cert.pem"
# TLS 私钥路径
key_path = "key.pem"

# 认证密钥（客户端必须提供相同的密钥才能连接）
# 请修改为你自己的强密码！
auth_key = "your-secret-auth-key-change-me"
"#
        }
        "client" => {
            r#"[client]
# 服务器地址
server_addr = "example.com"
# 服务器端口
server_port = 8443

# 是否跳过证书验证（仅用于测试，生产环境请设置为 false）
skip_verify = false

# CA 证书路径（可选，如果使用自签名证书）
# ca_cert_path = "ca.pem"

# 认证密钥（必须与服务器配置中的密钥一致）
# 请修改为你自己的强密码！
auth_key = "your-secret-auth-key-change-me"

# 代理配置列表
[[proxies]]
name = "web"
# 服务器发布端口（外部访问该端口）
publish_port = 8080
# 客户端本地服务端口（转发到该端口）
local_port = 3000

# 可以添加多个代理
# [[proxies]]
# name = "ssh"
# publish_port = 2222
# local_port = 22
"#
        }
        _ => unreachable!(),
    };

    if let Some(path) = output {
        std::fs::write(path, content)
            .with_context(|| format!("Failed to write config to {}", path))?;
        println!("Generated {} configuration file: {}", config_type, path);
    } else {
        println!("{}", content);
    }

    Ok(())
}
