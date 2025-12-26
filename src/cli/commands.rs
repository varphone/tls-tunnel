use anyhow::Result;
use tokio_rustls::{TlsAcceptor, TlsConnector};
use tracing::info;

use crate::{client, config::AppConfig, server, tls, top, transport};

use super::cert;
use super::config::{check_config, check_config_file_permissions, expand_path};
use super::{service, template};

/// Execute CLI commands
pub async fn execute_command(cli: &super::Cli) -> Result<()> {
    use super::Commands;

    match &cli.command {
        Commands::Check { config, format } => {
            let config_path = expand_path(config)?;
            check_config(&config_path, format)?;
        }
        Commands::Template {
            template_type,
            output,
        } => {
            template::generate_config_template(template_type, output.as_deref())?;
        }
        Commands::Cert {
            cert_out,
            key_out,
            common_name,
            alt_names,
        } => {
            cert::generate_certificate(cert_out, key_out, common_name, alt_names)?;
        }
        Commands::Register {
            service_type,
            config,
            name,
            exec,
        } => {
            let config_path = expand_path(config)?;
            service::register_systemd_service(
                service_type,
                &config_path,
                name.as_deref(),
                exec.as_deref(),
            )?;
        }
        Commands::Unregister { service_type, name } => {
            service::unregister_systemd_service(service_type, name.as_deref())?;
        }
        Commands::Server { config } => {
            run_server(config).await?;
        }
        Commands::Client { config } => {
            run_client(config).await?;
        }
        Commands::Top {
            config,
            url,
            interval,
        } => {
            run_top(config.as_deref(), url.as_deref(), *interval).await?;
        }
    }

    Ok(())
}

/// Run TLS tunnel server
async fn run_server(config: &str) -> Result<()> {
    let config_path = expand_path(config)?;

    // 检查配置文件权限
    check_config_file_permissions(&config_path)?;

    info!("Loading server configuration from: {}", config_path);
    let server_config = AppConfig::load_server_config(&config_path)?;

    // Load TLS configuration
    let (cert_path, key_path) = cert::ensure_server_certs(&server_config)?;

    // Set ALPN protocols based on transport type
    let alpn_protocols = if server_config.transport == transport::TransportType::Http2 {
        Some(vec![b"h2".to_vec()])
    } else {
        None
    };

    let tls_config = tls::load_server_config_with_alpn(&cert_path, &key_path, alpn_protocols)?;
    let acceptor = TlsAcceptor::from(tls_config);

    // Run server
    server::run_server(server_config, acceptor).await?;

    Ok(())
}

/// Run TLS tunnel client
async fn run_client(config: &str) -> Result<()> {
    let config_path = expand_path(config)?;

    // 检查配置文件权限
    check_config_file_permissions(&config_path)?;

    info!("Loading client configuration from: {}", config_path);
    let client_config = AppConfig::load_client_config(&config_path)?;

    // Load TLS configuration
    // Set ALPN protocols based on transport type
    let alpn_protocols = if client_config.client.transport == transport::TransportType::Http2 {
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

    // Run client
    client::run_client(client_config, connector).await?;

    Ok(())
}

/// Run statistics dashboard
async fn run_top(config: Option<&str>, url: Option<&str>, interval: u64) -> Result<()> {
    let stats_url = if let Some(config_path) = config {
        // 从配置文件读取统计服务器地址
        let config_path = expand_path(config_path)?;
        info!("Loading server configuration from: {}", config_path);
        let server_config = AppConfig::load_server_config(&config_path)?;

        let stats_port = server_config.stats_port.ok_or_else(|| {
            anyhow::anyhow!("stats_port is not configured in the server configuration file")
        })?;

        let stats_addr = server_config
            .stats_addr
            .as_ref()
            .filter(|s| !s.trim().is_empty())
            .cloned()
            .unwrap_or_else(|| server_config.bind_addr.clone());

        format!("http://{}:{}", stats_addr, stats_port)
    } else if let Some(url) = url {
        // 直接使用提供的 URL
        url.to_string()
    } else {
        anyhow::bail!("Either --config or --url must be provided");
    };

    info!("Connecting to statistics server: {}", stats_url);
    // Run statistics dashboard
    top::run_dashboard(stats_url, interval).await?;

    Ok(())
}
