use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug, Clone)]
#[command(
    name = "megatem",
    version = "v1.8.0",
    about = "Bulk MEGA account creator and manager",
    after_help = "Examples:\n  megatem                       Open the interactive menu\n  megatem -a 4 -sh               Create one account visibly\n  megatem -l 10 -j 4             Create 10 accounts with 4 workers\n  megatem --keepalive --prune -y Check accounts and prune dead ones\n  megatem --export-bitwarden     Export all accounts to Bitwarden CSV"
)]
pub struct Cli {
    /// Registration attempts per provider
    #[arg(short = 'a', long, default_value_t = 4)]
    pub attempts: u32,

    /// Check saved accounts and refresh lastLogin
    #[arg(long)]
    pub keepalive: bool,

    /// Create this many accounts sequentially
    #[arg(short = 'l', long = "loop")]
    pub loop_count: Option<u64>,

    /// Number of accounts to create in parallel
    #[arg(short = 'j', long, default_value_t = 1)]
    pub parallel: u32,

    /// Run parallel registrations in isolated processes
    #[arg(long)]
    pub isolate: bool,

    /// Keep Chromium profiles after registration
    #[arg(long)]
    pub keep_profiles: bool,

    /// Show disk usage
    #[arg(long)]
    pub disk_usage: bool,

    /// Remove temporary profiles, partial downloads and old backups
    #[arg(long)]
    pub clean: bool,

    /// Resume a previous loop using loop_state.json
    #[arg(long)]
    pub resume: bool,

    /// Email provider (mailtm or guerrillamail)
    #[arg(long)]
    pub provider: Option<String>,

    /// Seconds to wait for the confirmation email
    #[arg(long)]
    pub mail_timeout: Option<u64>,

    /// Upload this file after creating an account
    #[arg(short = 'f', long)]
    pub file: Option<PathBuf>,

    /// Upload every file in this directory after creating an account
    #[arg(long)]
    pub upload_dir: Option<PathBuf>,

    /// Request a public link for uploads
    #[arg(short = 'p', long)]
    pub public: bool,

    /// List files in the cloud drive
    #[arg(long)]
    pub list_cloud: bool,

    /// Download a cloud file by id
    #[arg(long)]
    pub download_cloud: Option<String>,

    /// Destination directory for downloads
    #[arg(long, default_value = ".")]
    pub download_dest: PathBuf,

    /// Append new accounts to credentials/accounts.csv
    #[arg(long)]
    pub export_csv: bool,

    /// Append new accounts to credentials/accounts.jsonl
    #[arg(long)]
    pub export_jsonl: bool,

    /// Export all accounts for Bitwarden
    #[arg(long)]
    pub export_bitwarden: bool,

    /// Export all accounts for 1Password
    #[arg(long)]
    pub export_onepassword: bool,

    /// Export all accounts for KeePass
    #[arg(long)]
    pub export_keepass: bool,

    /// Write credentials.txt
    #[arg(short = 'e', long)]
    pub extract: bool,

    /// Proxy URL
    #[arg(long)]
    pub proxy: Option<String>,

    /// File containing one proxy per line
    #[arg(long)]
    pub proxy_file: Option<String>,

    /// Rotate proxies per attempt
    #[arg(long)]
    pub proxy_per_attempt: bool,

    /// Fetch a proxy from a URL
    #[arg(long)]
    pub proxy_url: Option<String>,

    /// Prune accounts that fail authentication
    #[arg(long)]
    pub prune: bool,

    /// Repeat keepalive every N hours
    #[arg(long)]
    pub interval: Option<f64>,

    /// Answer yes to confirmation prompts
    #[arg(short = 'y', long)]
    pub yes: bool,

    /// Verbose output
    #[arg(short = 'v', long)]
    pub verbose: bool,

    /// Suppress non-essential output
    #[arg(long)]
    pub quiet: bool,

    /// Print a health report
    #[arg(long)]
    pub health: bool,

    /// Emit machine-readable JSON (health)
    #[arg(long)]
    pub json: bool,

    /// Show the browser window
    #[arg(long)]
    pub visible: bool,

    /// Config profile name
    #[arg(long)]
    pub profile: Option<String>,

    /// Encryption password for credential fields
    #[arg(long)]
    pub encryption_password: Option<String>,

    /// Webhook URL for registration events
    #[arg(long)]
    pub webhook_url: Option<String>,

    /// Send desktop notifications
    #[arg(long)]
    pub notify: bool,

    /// Keepalive workers (0 = automatic)
    #[arg(long, default_value_t = 0)]
    pub workers: usize,

    /// Browser executable
    #[arg(long)]
    pub browser: Option<String>,

    /// Check GitHub for a newer release
    #[arg(long)]
    pub update: bool,
}

/// Accept the multi-character short flags the Python CLI supports.
pub fn normalize_args() -> Vec<String> {
    std::env::args()
        .map(|arg| match arg.as_str() {
            "-ka" => "--keepalive".to_string(),
            "-sh" => "--visible".to_string(),
            "-csv" => "--export-csv".to_string(),
            other => other.to_string(),
        })
        .collect()
}
