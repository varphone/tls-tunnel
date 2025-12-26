use anyhow::Result;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::info;

use crate::{config, tls};

/// Generate self-signed TLS certificate
pub fn generate_certificate(
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

    tls::generate_self_signed_cert(common_name, &sans, Path::new(cert_out), Path::new(key_out))?;

    println!("Generated self-signed certificate: {}", cert_out);
    println!("Generated private key: {}", key_out);

    Ok(())
}

/// Ensure server TLS certificates are available; generate self-signed certificates at runtime if not configured
pub fn ensure_server_certs(config: &config::ServerConfig) -> Result<(PathBuf, PathBuf)> {
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

            // Use bind address as CN/SAN (fallback to localhost if 0.0.0.0)
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
