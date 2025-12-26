use anyhow::{Context, Result};
use serde::Serialize;
use std::path::{Path, PathBuf};

use crate::config::AppConfig;

/// 检查配置文件权限（仅Unix系统）
#[cfg(unix)]
pub fn check_config_file_permissions(config_path: &str) -> Result<()> {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use tracing::warn;

    let metadata = fs::metadata(config_path)
        .with_context(|| format!("Failed to read metadata for config file: {}", config_path))?;
    let permissions = metadata.permissions();
    let mode = permissions.mode();

    // 检查是否其他用户可读（o+r = 0o004）
    if mode & 0o004 != 0 {
        warn!(
            "⚠️  SECURITY WARNING: Config file '{}' is readable by others (permissions: {:o})\n\
             This file may contain sensitive information (auth_key).\n\
             RECOMMENDATION: chmod 600 {}",
            config_path,
            mode & 0o777,
            config_path
        );
    }

    // 检查是否组用户可读（g+r = 0o040）
    if mode & 0o040 != 0 {
        warn!(
            "⚠️  SECURITY WARNING: Config file '{}' is readable by group (permissions: {:o})\n\
             RECOMMENDATION: chmod 600 {}",
            config_path,
            mode & 0o777,
            config_path
        );
    }

    Ok(())
}

/// Windows系统不进行权限检查
#[cfg(not(unix))]
pub fn check_config_file_permissions(_config_path: &str) -> Result<()> {
    // Windows 权限模型不同，不进行简单的模式位检查
    Ok(())
}

#[derive(Serialize)]
struct CheckResult {
    valid: bool,
    config_type: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    warnings: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    details: serde_json::Value,
}

/// Expand path with tilde (~) and make it absolute
pub fn expand_path(path: &str) -> Result<String> {
    let expanded = shellexpand::tilde(path);
    let path_buf = PathBuf::from(expanded.as_ref());

    if path_buf.is_absolute() {
        Ok(expanded.into_owned())
    } else {
        // Convert relative path to absolute
        std::env::current_dir()
            .context("Failed to get current directory")?
            .join(&path_buf)
            .to_str()
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow::anyhow!("Invalid path: {}", path))
    }
}

/// Check configuration file format
pub fn check_config(config_path: &str, format: &str) -> Result<()> {
    let path = Path::new(config_path);

    // Check if file exists
    if !path.exists() {
        if format == "json" {
            let result = CheckResult {
                valid: false,
                config_type: "unknown".to_string(),
                warnings: vec![],
                error: Some(format!("Configuration file not found: {}", config_path)),
                details: serde_json::json!({}),
            };
            println!("{}", serde_json::to_string_pretty(&result)?);
        } else {
            println!("✗ Configuration file not found: {}", config_path);
        }
        anyhow::bail!("Configuration file not found: {}", config_path);
    }

    if format == "text" {
        println!("Checking configuration file: {}\n", config_path);
    }

    // Try to load as server configuration
    if let Ok(server_config) = AppConfig::load_server_config(config_path) {
        let mut warnings = Vec::new();
        let mut details = serde_json::json!({
            "bind_addr": server_config.bind_addr,
            "bind_port": server_config.bind_port,
            "auth_key_length": server_config.auth_key.len(),
        });

        match (&server_config.cert_path, &server_config.key_path) {
            (Some(cert), Some(key)) => {
                details["cert_path"] = serde_json::json!(cert);
                details["key_path"] = serde_json::json!(key);

                if !cert.exists() {
                    warnings.push(format!("Certificate file not found: {:?}", cert));
                }
                if !key.exists() {
                    warnings.push(format!("Key file not found: {:?}", key));
                }
            }
            (None, None) => {
                details["cert_mode"] = serde_json::json!("auto-generate");
            }
            _ => {
                if format == "json" {
                    let result = CheckResult {
                        valid: false,
                        config_type: "server".to_string(),
                        warnings,
                        error: Some(
                            "cert_path and key_path must both be set, or both omitted".to_string(),
                        ),
                        details,
                    };
                    println!("{}", serde_json::to_string_pretty(&result)?);
                } else {
                    println!("✗ cert_path/key_path mismatch: set both, or leave both empty to auto-generate");
                }
                anyhow::bail!(
                    "cert_path and key_path must both be set, or both omitted to auto-generate"
                );
            }
        }

        if format == "json" {
            let result = CheckResult {
                valid: true,
                config_type: "server".to_string(),
                warnings,
                error: None,
                details,
            };
            println!("{}", serde_json::to_string_pretty(&result)?);
        } else {
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
                _ => {}
            }
            println!("\n✓ Server configuration is valid!");
        }
        return Ok(());
    }

    // Try to load as client configuration
    match AppConfig::load_client_config(config_path) {
        Ok(client_config) => {
            let mut warnings = Vec::new();
            let mut proxies_info = Vec::new();

            for proxy in &client_config.proxies {
                proxies_info.push(serde_json::json!({
                    "name": proxy.name,
                    "publish_port": proxy.publish_port,
                    "local_port": proxy.local_port,
                }));
            }

            if client_config.proxies.is_empty() {
                warnings.push("No proxy configurations defined".to_string());
            }

            let mut details = serde_json::json!({
                "server_addr": client_config.client.server_addr,
                "server_port": client_config.client.server_port,
                "skip_verify": client_config.client.skip_verify,
                "auth_key_length": client_config.client.auth_key.len(),
                "proxies_count": client_config.proxies.len(),
                "proxies": proxies_info,
            });

            if let Some(ref ca_path) = client_config.client.ca_cert_path {
                details["ca_cert_path"] = serde_json::json!(ca_path);
                if !ca_path.exists() {
                    warnings.push(format!("CA certificate file not found: {:?}", ca_path));
                }
            }

            if format == "json" {
                let result = CheckResult {
                    valid: true,
                    config_type: "client".to_string(),
                    warnings,
                    error: None,
                    details,
                };
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
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
            }
            Ok(())
        }
        Err(e) => {
            if format == "json" {
                let result = CheckResult {
                    valid: false,
                    config_type: "unknown".to_string(),
                    warnings: vec![],
                    error: Some(format!("{:#}", e)),
                    details: serde_json::json!({}),
                };
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                println!("✗ Configuration validation failed!");
                println!("\nError details:");
                println!("{:#}", e);

                // Provide hints for common issues
                println!("\nCommon issues:");
                println!("  1. Check TOML syntax (brackets, quotes, commas)");
                println!("  2. Ensure all required fields are present");
                println!("  3. Verify field names are spelled correctly");
                println!("  4. Check that paths use forward slashes or escaped backslashes");
                println!("  5. Ensure port numbers are valid (1-65535)");
                println!("  6. For server config: [server] section with bind_addr, bind_port, auth_key, and optional cert_path/key_path (omit both to auto-generate)");
                println!("  7. For client config: [client] section with server_addr, server_port, auth_key, and [[proxies]] sections");
            }

            Err(e)
        }
    }
}
