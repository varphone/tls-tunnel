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
    /// 生成示例配置文件
    Generate {
        /// 配置类型 (server 或 client)
        #[arg(value_parser = ["server", "client"])]
        config_type: String,
        
        /// 输出文件路径
        #[arg(short, long)]
        output: Option<String>,
    },
    /// 检查配置文件格式是否正确
    Check {
        /// 配置文件路径
        #[arg(short, long)]
        config: String,
    },
}
