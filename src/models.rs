use serde::{Deserialize, Serialize};

/// One saved account. Field names match the Python implementation's JSON.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Credentials {
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub email_password: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub tags: String,
    #[serde(default)]
    pub last_login: f64,
}

impl Credentials {
    pub fn new(email: impl Into<String>, password: impl Into<String>) -> Self {
        Self {
            email: email.into(),
            password: password.into(),
            ..Default::default()
        }
    }
}

pub const CURRENT_SCHEMA: u32 = 3;

/// Application configuration. Field names match `config.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    #[serde(default = "current_schema")]
    pub schema_version: u32,
    #[serde(default)]
    pub executable_path: String,
    #[serde(default)]
    pub account_format: String,
    #[serde(default)]
    pub proxy: String,
    #[serde(default)]
    pub proxy_file: String,
    #[serde(default)]
    pub proxy_per_attempt: bool,
    #[serde(default = "default_attempts")]
    pub max_attempts: u32,
    #[serde(default)]
    pub csv_export: bool,
    #[serde(default)]
    pub jsonl_export: bool,
    #[serde(default)]
    pub visible_browser: bool,
    #[serde(default = "default_provider")]
    pub email_provider: String,
    #[serde(default = "default_mail_timeout")]
    pub mail_timeout: u64,
    #[serde(default)]
    pub quiet: bool,
    #[serde(default)]
    pub webhook_url: String,
    #[serde(default)]
    pub encryption_password: String,
}

fn current_schema() -> u32 {
    CURRENT_SCHEMA
}
fn default_attempts() -> u32 {
    4
}
fn default_provider() -> String {
    "mailtm".to_string()
}
fn default_mail_timeout() -> u64 {
    120
}

impl Default for Config {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_SCHEMA,
            executable_path: String::new(),
            account_format: String::new(),
            proxy: String::new(),
            proxy_file: String::new(),
            proxy_per_attempt: false,
            max_attempts: default_attempts(),
            csv_export: false,
            jsonl_export: false,
            visible_browser: false,
            email_provider: default_provider(),
            mail_timeout: default_mail_timeout(),
            quiet: false,
            webhook_url: String::new(),
            encryption_password: String::new(),
        }
    }
}

/// All keys the Python version understands.
pub const CONFIG_KEYS: &[&str] = &[
    "schemaVersion",
    "executablePath",
    "accountFormat",
    "proxy",
    "proxyFile",
    "proxyPerAttempt",
    "maxAttempts",
    "csvExport",
    "jsonlExport",
    "visibleBrowser",
    "emailProvider",
    "mailTimeout",
    "quiet",
    "webhookUrl",
    "encryptionPassword",
];
