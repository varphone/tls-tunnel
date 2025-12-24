use anyhow::{Context, Result};
use rcgen::generate_simple_self_signed;
use rustls::pki_types::CertificateDer;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::sync::Arc;
use tokio_rustls::rustls;

/// 加载服务器 TLS 配置
#[allow(dead_code)]
pub fn load_server_config(cert_path: &Path, key_path: &Path) -> Result<Arc<rustls::ServerConfig>> {
    load_server_config_with_alpn(cert_path, key_path, None)
}

/// 加载服务器 TLS 配置，支持ALPN
pub fn load_server_config_with_alpn(
    cert_path: &Path,
    key_path: &Path,
    alpn_protocols: Option<Vec<Vec<u8>>>,
) -> Result<Arc<rustls::ServerConfig>> {
    // 加载证书
    let cert_file = File::open(cert_path)
        .with_context(|| format!("Failed to open cert file: {:?}", cert_path))?;
    let mut cert_reader = BufReader::new(cert_file);
    let certs: Vec<CertificateDer> = rustls_pemfile::certs(&mut cert_reader)
        .collect::<Result<Vec<_>, _>>()
        .context("Failed to parse certificates")?;

    // 加载私钥
    let key_file =
        File::open(key_path).with_context(|| format!("Failed to open key file: {:?}", key_path))?;
    let mut key_reader = BufReader::new(key_file);

    let key = rustls_pemfile::private_key(&mut key_reader)
        .context("Failed to parse private key")?
        .context("No private key found")?;

    // 创建 TLS 配置
    let mut config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .context("Failed to create server config")?;

    // 设置 ALPN 协议
    if let Some(protocols) = alpn_protocols {
        config.alpn_protocols = protocols;
    }

    Ok(Arc::new(config))
}

/// 加载客户端 TLS 配置
#[allow(dead_code)]
pub fn load_client_config(
    ca_cert_path: Option<&Path>,
    skip_verify: bool,
) -> Result<Arc<rustls::ClientConfig>> {
    load_client_config_with_alpn(ca_cert_path, skip_verify, None)
}

/// 加载客户端 TLS 配置，支持ALPN
pub fn load_client_config_with_alpn(
    ca_cert_path: Option<&Path>,
    skip_verify: bool,
    alpn_protocols: Option<Vec<Vec<u8>>>,
) -> Result<Arc<rustls::ClientConfig>> {
    let mut root_store = rustls::RootCertStore::empty();

    if let Some(ca_path) = ca_cert_path {
        // 加载自定义 CA 证书
        let ca_file = File::open(ca_path)
            .with_context(|| format!("Failed to open CA cert file: {:?}", ca_path))?;
        let mut ca_reader = BufReader::new(ca_file);
        let ca_certs: Vec<CertificateDer> = rustls_pemfile::certs(&mut ca_reader)
            .collect::<Result<Vec<_>, _>>()
            .context("Failed to parse CA certificates")?;

        for cert in ca_certs {
            root_store
                .add(cert)
                .context("Failed to add CA certificate")?;
        }
    } else if !skip_verify {
        // 使用系统 CA 证书
        let native_certs = rustls_native_certs::load_native_certs();
        for cert in native_certs.certs {
            root_store.add(cert).ok();
        }
    }

    let mut config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();

    // 如果跳过证书验证（仅用于测试）
    if skip_verify {
        config
            .dangerous()
            .set_certificate_verifier(Arc::new(NoCertificateVerification));
    }

    // 设置 ALPN 协议
    if let Some(protocols) = alpn_protocols {
        config.alpn_protocols = protocols;
    }

    Ok(Arc::new(config))
}

/// 生成自签名证书和私钥并写入指定路径
pub fn generate_self_signed_cert(
    common_name: &str,
    alt_names: &[String],
    cert_out: &Path,
    key_out: &Path,
) -> Result<()> {
    // rcgen 至少需要一个 SAN；确保包含 CN
    let mut names: Vec<String> = if alt_names.is_empty() {
        vec![common_name.to_string()]
    } else {
        alt_names.to_vec()
    };

    if !names.iter().any(|n| n == common_name) {
        names.push(common_name.to_string());
    }

    let cert =
        generate_simple_self_signed(names).context("Failed to generate self-signed certificate")?;
    // rcgen 0.14 returns CertifiedKey; cert field carries der, signing_key holds private key
    let cert_pem = cert.cert.pem();
    let key_pem = cert.signing_key.serialize_pem();

    std::fs::write(cert_out, cert_pem)
        .with_context(|| format!("Failed to write certificate to {:?}", cert_out))?;
    std::fs::write(key_out, key_pem)
        .with_context(|| format!("Failed to write private key to {:?}", key_out))?;

    Ok(())
}

/// 不验证证书的验证器（仅用于测试）
#[derive(Debug)]
struct NoCertificateVerification;

impl rustls::client::danger::ServerCertVerifier for NoCertificateVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer,
        _intermediates: &[CertificateDer],
        _server_name: &rustls::pki_types::ServerName,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![
            rustls::SignatureScheme::RSA_PKCS1_SHA256,
            rustls::SignatureScheme::RSA_PKCS1_SHA384,
            rustls::SignatureScheme::RSA_PKCS1_SHA512,
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::ECDSA_NISTP384_SHA384,
            rustls::SignatureScheme::ECDSA_NISTP521_SHA512,
            rustls::SignatureScheme::ED25519,
            rustls::SignatureScheme::RSA_PSS_SHA256,
            rustls::SignatureScheme::RSA_PSS_SHA384,
            rustls::SignatureScheme::RSA_PSS_SHA512,
        ]
    }
}
