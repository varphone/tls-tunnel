use anyhow::Result;
use clap::Parser;
use tls_tunnel::cli::{self, Cli};
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialize logging based on verbosity level or RUST_LOG env var
    // Priority: RUST_LOG > --verbose flag
    let default_log_level = match cli.verbose {
        0 => "off",
        1 => "info",
        2 => "debug",
        _ => "trace",
    };

    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(default_log_level));

    // 检测是否在 systemd 环境中运行
    // systemd 会设置 INVOCATION_ID 或 JOURNAL_STREAM 环境变量
    let is_systemd =
        std::env::var("INVOCATION_ID").is_ok() || std::env::var("JOURNAL_STREAM").is_ok();

    let fmt_layer = tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_target(false)
        .with_thread_ids(false)
        .with_file(false)
        .with_line_number(false);

    // systemd 自带时间戳，不需要重复输出
    if is_systemd {
        fmt_layer.without_time().init();
    } else {
        fmt_layer.init();
    }

    // Display version information
    info!("TLS Tunnel v{}", env!("CARGO_PKG_VERSION"));

    // Execute command
    cli::execute_command(&cli).await?;

    Ok(())
}
