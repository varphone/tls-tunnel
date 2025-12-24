mod cli;
mod client;
mod config;
mod connection_pool;
mod server;
mod tls;

use anyhow::{Context, Result};
use clap::Parser;
use cli::{Cli, Commands};
use config::AppConfig;
use tokio_rustls::{TlsAcceptor, TlsConnector};
use tracing::info;

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
        } => {
            generate_example_config(config_type, output.as_deref())?;
            return Ok(());
        }
        Commands::Server { config } => {
            info!("Loading server configuration from: {}", config);
            let server_config = AppConfig::load_server_config(config)?;

            // 加载 TLS 配置
            let tls_config =
                tls::load_server_config(&server_config.cert_path, &server_config.key_path)?;
            let acceptor = TlsAcceptor::from(tls_config);

            // 运行服务器
            server::run_server(server_config, acceptor).await?;
        }
        Commands::Client { config } => {
            info!("Loading client configuration from: {}", config);
            let client_config = AppConfig::load_client_config(config)?;

            // 加载 TLS 配置
            let tls_config = tls::load_client_config(
                client_config.client.ca_cert_path.as_deref(),
                client_config.client.skip_verify,
            )?;
            let connector = TlsConnector::from(tls_config);

            // 运行客户端
            client::run_client(client_config, connector).await?;
        }
    }

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
    match AppConfig::load_server_config(config_path) {
        Ok(server_config) => {
            println!("✓ Configuration type: Server");
            println!("✓ Bind address: {}", server_config.bind_addr);
            println!("✓ Bind port: {}", server_config.bind_port);
            println!("✓ Certificate path: {:?}", server_config.cert_path);
            println!("✓ Key path: {:?}", server_config.key_path);
            println!("✓ Auth key: {} characters", server_config.auth_key.len());

            // 检查证书文件是否存在
            if !server_config.cert_path.exists() {
                println!(
                    "⚠ Warning: Certificate file not found: {:?}",
                    server_config.cert_path
                );
            } else {
                println!("✓ Certificate file exists");
            }

            // 检查密钥文件是否存在
            if !server_config.key_path.exists() {
                println!(
                    "⚠ Warning: Key file not found: {:?}",
                    server_config.key_path
                );
            } else {
                println!("✓ Key file exists");
            }

            println!("\n✓ Server configuration is valid!");
            return Ok(());
        }
        Err(_) => {}
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
            return Ok(());
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
            println!("  6. For server config: [server] section with bind_addr, bind_port, cert_path, key_path, auth_key");
            println!("  7. For client config: [client] section with server_addr, server_port, auth_key, and [[proxies]] sections");

            return Err(e);
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
