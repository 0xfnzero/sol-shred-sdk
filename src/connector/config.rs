use clap::Parser;
use std::collections::HashMap;
use std::path::PathBuf;

/// Command-line arguments
#[derive(Parser, Debug, Clone)]
#[command(name = "jetstream-example")]
#[command(about = "JetStream Example Client", long_about = None)]
pub struct ClientConfig {
    /// Jito gRPC URL
    #[arg(short = 'j', long, default_value = "PLACE_URL_HERE")]
    pub jetstream_grpc_url: String,

    /// X token
    #[arg(short = 'x', long)]
    pub x_token: Option<String>,

    /// Filter config file path (JSON format)
    #[arg(short = 'f', long)]
    pub filter_config_path: Option<PathBuf>,

    /// Include accounts (comma-separated Solana pubkeys)
    #[arg(short = 'i', long, value_delimiter = ',')]
    pub include_accounts: Option<Vec<String>>,

    /// Exclude accounts (comma-separated Solana pubkeys)
    #[arg(short = 'e', long, value_delimiter = ',')]
    pub exclude_accounts: Option<Vec<String>>,

    /// Required accounts (comma-separated Solana pubkeys)
    #[arg(short = 'r', long, value_delimiter = ',')]
    pub required_accounts: Option<Vec<String>>,

    /// Enable parsed instruction streaming
    #[arg(short = 'p', long, action = clap::ArgAction::SetTrue)]
    pub parsed_enabled: bool,
}

/// Represents a transaction filter configuration
#[derive(Debug, Clone, serde::Deserialize)]
pub struct FilterConfig {
    pub filters: HashMap<String, Filter>,
}

/// Individual filter settings
#[derive(Debug, Clone, serde::Deserialize)]
pub struct Filter {
    #[serde(default)]
    pub account_include: Vec<String>,
    #[serde(default)]
    pub account_exclude: Vec<String>,
    #[serde(default)]
    pub account_required: Vec<String>,
}
