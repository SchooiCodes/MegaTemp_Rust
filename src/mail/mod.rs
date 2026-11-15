use anyhow::{bail, Result};
use rand::Rng;
use std::time::Duration;

use crate::util::random_alnum;

pub mod guerrilla;
pub mod mailtm;

#[derive(Debug, Clone, Default)]
pub struct Mailbox {
    pub provider: String,
    pub address: String,
    pub token: String,
    pub secret: String,
}

/// A provider refused us for rate limiting; retrying the same provider is pointless.
#[derive(Debug)]
pub struct RateLimited {
    pub provider: &'static str,
    pub detail: String,
}

impl std::fmt::Display for RateLimited {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} is rate limiting this IP: {}", self.provider, self.detail)
    }
}

impl std::error::Error for RateLimited {}

/// True when any error in the chain is a provider rate limit.
pub fn is_rate_limited(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| cause.is::<RateLimited>())
}

#[derive(Debug, Clone, Default)]
pub struct Message {
    pub mail_id: String,
    pub mail_from: String,
    pub mail_subject: String,
    pub mail_body: String,
}

#[derive(Clone)]
pub enum MailProvider {
    MailTm(mailtm::MailTm),
    Guerrilla(guerrilla::Guerrilla),
}

impl MailProvider {
    pub fn from_name(name: &str) -> Result<Self> {
        match name.to_lowercase().as_str() {
            "mailtm" | "mail.tm" => Ok(Self::MailTm(mailtm::MailTm::new()?)),
            "guerrillamail" | "guerrilla" => Ok(Self::Guerrilla(guerrilla::Guerrilla::new()?)),
            other => bail!(
                "unknown email provider '{other}' (valid: {} )",
                valid_providers().join(", ")
            ),
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::MailTm(_) => "mailtm",
            Self::Guerrilla(_) => "guerrillamail",
        }
    }

    pub async fn create(&self) -> Result<Mailbox> {
        match self {
            Self::MailTm(p) => p.create().await,
            Self::Guerrilla(p) => p.create().await,
        }
    }

    pub async fn login(&self, address: &str, secret: &str) -> Result<Mailbox> {
        match self {
            Self::MailTm(p) => p.login(address, secret).await,
            Self::Guerrilla(p) => p.login(address).await,
        }
    }

    pub async fn latest_message(&self, mailbox: &Mailbox) -> Result<Option<Message>> {
        match self {
            Self::MailTm(p) => p.latest_message(mailbox).await,
            Self::Guerrilla(p) => p.latest_message(mailbox).await,
        }
    }
}

pub fn valid_providers() -> Vec<&'static str> {
    vec!["mailtm", "guerrillamail"]
}

pub fn fallback_provider(name: &str) -> &'static str {
    if name.eq_ignore_ascii_case("mailtm") {
        "guerrillamail"
    } else {
        "mailtm"
    }
}

pub fn generate_address_local() -> String {
    format!("mt{}", random_alnum(12))
}

/// Poll a mailbox until a message arrives or the timeout expires.
pub async fn wait_for_message(
    provider: &MailProvider,
    mailbox: &Mailbox,
    timeout_secs: u64,
) -> Result<Message> {
    let attempts = ((timeout_secs as f64 / 2.0).round() as u64).max(1);
    for attempt in 1..=attempts {
        let base = (1.0 + 0.25 * (attempt.saturating_sub(1)) as f64).min(3.0);
        let jitter = rand::thread_rng().gen_range(0.8..1.2);
        tokio::time::sleep(Duration::from_secs_f64(base * jitter)).await;
        match provider.latest_message(mailbox).await {
            Ok(Some(message)) => return Ok(message),
            Ok(None) => continue,
            Err(e) => {
                let delay = 0.3 + (attempt % 5) as f64 * 0.25;
                tokio::time::sleep(Duration::from_secs_f64(delay)).await;
                if attempt == attempts {
                    return Err(e);
                }
            }
        }
    }
    bail!("no confirmation email arrived within {timeout_secs}s")
}

/// Retry helper shared by the HTTP providers.
pub(crate) async fn with_retry<F, Fut, T>(mut action: F) -> Result<T>
where
    F: FnMut(u32) -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    let mut last = None;
    for n in 1..=10u32 {
        match action(n).await {
            Ok(v) => return Ok(v),
            Err(e) => {
                last = Some(e);
                let delay = if n == 1 {
                    0.3
                } else {
                    (2u64.pow(n - 1) + rand::thread_rng().gen_range(0..2)).min(10) as f64
                };
                tokio::time::sleep(Duration::from_secs_f64(delay)).await;
            }
        }
    }
    Err(last.unwrap_or_else(|| anyhow::anyhow!("request failed")))
}
