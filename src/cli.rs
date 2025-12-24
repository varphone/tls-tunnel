use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "tls-tunnel")]
#[command(author, version, about = "TLS-based reverse proxy tunnel", long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    /// 日志级别 (trace, debug, info, warn, error)
    #[arg(short, long, default_value = "info", global = true)]
    pub log_level: String,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// 运行服务器模式
    Server {
        /// 配置文件路径
        #[arg(short, long, default_value = "server.toml")]
        config: String,
    },
    /// 运行客户端模式
    Client {
        /// 配置文件路径
        #[arg(short, long, default_value = "client.toml")]
        config: String,
    },
    /// 生成示例配置/证书/systemd 服务文件
    Generate {
        /// 配置类型 (server, client, cert, systemd)
        #[arg(value_parser = ["server", "client", "cert", "systemd"])]
        config_type: String,

        /// 输出文件路径
        #[arg(short, long)]
        output: Option<String>,

        /// 生成自签名证书的输出路径（cert.pem）
        #[arg(long, value_name = "PATH")]
        cert_out: Option<String>,

        /// 生成自签名私钥的输出路径（key.pem）
        #[arg(long, value_name = "PATH")]
        key_out: Option<String>,

        /// 证书的 Common Name
        #[arg(long, default_value = "localhost")]
        common_name: String,

        /// 证书的 SubjectAltName（用逗号分隔多个）
        #[arg(long, value_delimiter = ',', value_name = "DNS,...")]
        alt_names: Vec<String>,

        /// 生成 systemd 服务文件的输出路径
        #[arg(long, value_name = "PATH")]
        systemd_out: Option<String>,

        /// systemd 服务使用的配置文件路径
        #[arg(long, value_name = "PATH")]
        service_config: Option<String>,

        /// systemd 服务使用的可执行文件路径
        #[arg(long, value_name = "PATH")]
        service_exec: Option<String>,
    },
    /// 检查配置文件格式是否正确
    Check {
        /// 配置文件路径
        #[arg(short, long)]
        config: String,
    },
}
