use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "tls-tunnel")]
#[command(author, version, about = "TLS-based reverse proxy tunnel", long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    /// Increase logging verbosity (default: off, -v: info, -vv: debug, -vvv+: trace)
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    pub verbose: u8,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Run server mode
    Server {
        /// Configuration file path
        #[arg(short, long, default_value = "server.toml")]
        config: String,
    },
    /// Run client mode
    Client {
        /// Configuration file path
        #[arg(short, long, default_value = "client.toml")]
        config: String,
    },
    /// Generate configuration template
    Template {
        /// Template type (server, client)
        #[arg(value_parser = ["server", "client"])]
        template_type: String,

        /// Output file path (prints to stdout if not specified)
        #[arg(short, long)]
        output: Option<String>,
    },
    /// Generate self-signed TLS certificate
    Cert {
        /// Certificate output path
        #[arg(long, default_value = "cert.pem")]
        cert_out: String,

        /// Private key output path
        #[arg(long, default_value = "key.pem")]
        key_out: String,

        /// Certificate Common Name
        #[arg(long, default_value = "localhost")]
        common_name: String,

        /// Certificate SubjectAltName (comma-separated)
        #[arg(long, value_delimiter = ',', value_name = "DNS,...")]
        alt_names: Vec<String>,
    },
    /// Register as systemd service (Linux only)
    Register {
        /// Service type (server, client)
        #[arg(value_parser = ["server", "client"])]
        service_type: String,

        /// Configuration file path
        #[arg(short, long)]
        config: String,

        /// Service name (default: tls-tunnel-server or tls-tunnel-client)
        #[arg(short, long)]
        name: Option<String>,

        /// Executable path (default: current executable)
        #[arg(long)]
        exec: Option<String>,
    },
    /// Unregister systemd service (Linux only)
    Unregister {
        /// Service type (server, client)
        #[arg(value_parser = ["server", "client"])]
        service_type: String,

        /// Service name (default: tls-tunnel-server or tls-tunnel-client)
        #[arg(short, long)]
        name: Option<String>,
    },
    /// Check configuration file validity
    Check {
        /// Configuration file path
        #[arg(short, long)]
        config: String,
    },
}
