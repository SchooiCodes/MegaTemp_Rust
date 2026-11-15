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

/// A provider delivered MEGA's verification mail without a readable body.
///
/// Guerrilla Mail stores MEGA's HTML-only message as `mail_body: "<pre></pre>"`,
/// so the confirmation link can never be extracted; polling would just spin.
#[derive(Debug)]
pub struct UnusableMail {
    pub provider: String,
    pub detail: String,
}

impl std::fmt::Display for UnusableMail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} returned an unreadable verification email: {}",
            self.provider, self.detail
        )
    }
}

impl std::error::Error for UnusableMail {}

/// True when any error in the chain says the mailbox cannot render MEGA mail.
pub fn is_unusable(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| cause.is::<UnusableMail>())
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

/// A provider welcome mail is never the MEGA confirmation.
///
/// Guerrilla Mail drops a "Welcome to Guerrilla Mail" message into every new
/// inbox, so treating the newest message as the confirmation aborts the flow
/// before MEGA has even sent anything.
pub fn is_confirmation(message: &Message) -> bool {
    let subject = message.mail_subject.to_lowercase();
    if subject.contains("welcome to guerrilla") {
        return false;
    }
    let body = message.mail_body.to_lowercase();
    body.contains("mega.nz") || body.contains("mega.co.nz")
}

/// True when the message is plausibly MEGA's verification mail.
pub fn looks_like_mega(message: &Message) -> bool {
    let from = message.mail_from.to_lowercase();
    let subject = message.mail_subject.to_lowercase();
    from.contains("mega.io") || from.contains("mega.nz") || subject.contains("mega")
}

/// Poll a mailbox until a MEGA confirmation arrives or the timeout expires.
pub async fn wait_for_message(
    provider: &MailProvider,
    mailbox: &Mailbox,
    timeout_secs: u64,
) -> Result<Message> {
    let attempts = ((timeout_secs as f64 / 2.0).round() as u64).max(1);
    let mut skipped: Option<String> = None;
    let started = std::time::Instant::now();
    for attempt in 1..=attempts {
        let base = (1.0 + 0.25 * (attempt.saturating_sub(1)) as f64).min(3.0);
        let jitter = rand::thread_rng().gen_range(0.8..1.2);
        crate::console::status_line(&format!(
            "waiting for the confirmation email ({}s/{}s)",
            started.elapsed().as_secs(),
            timeout_secs
        ));
        tokio::time::sleep(Duration::from_secs_f64(base * jitter)).await;
        match provider.latest_message(mailbox).await {
            Ok(Some(message)) => {
                if is_confirmation(&message) {
                    crate::console::clear_status_line();
                    return Ok(message);
                }
                if looks_like_mega(&message) {
                    crate::console::clear_status_line();
                    return Err(UnusableMail {
                        provider: mailbox.provider.clone(),
                        detail: format!(
                            "'{}' arrived with an empty body, so the confirmation link is missing",
                            message.mail_subject
                        ),
                    }
                    .into());
                }
                skipped = Some(message.mail_subject.clone());
            }
            Ok(None) => continue,
            Err(e) => {
                let delay = 0.3 + (attempt % 5) as f64 * 0.25;
                tokio::time::sleep(Duration::from_secs_f64(delay)).await;
                if attempt == attempts {
                    crate::console::clear_status_line();
                    return Err(e);
                }
            }
        }
    }
    crate::console::clear_status_line();
    match skipped {
        Some(subject) => bail!(
            "no MEGA confirmation email arrived within {timeout_secs}s (newest message: {subject})"
        ),
        None => bail!("no confirmation email arrived within {timeout_secs}s"),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn message(subject: &str, body: &str) -> Message {
        Message {
            mail_subject: subject.to_string(),
            mail_body: body.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn welcome_mail_is_not_a_confirmation() {
        assert!(!is_confirmation(&message(
            "Welcome to Guerrilla Mail",
            "Thanks for using the service"
        )));
    }

    #[test]
    fn mega_link_counts_as_confirmation() {
        assert!(is_confirmation(&message(
            "Confirm your MEGA account",
            "https://mega.nz/#confirmAbCdEf"
        )));
        assert!(!is_confirmation(&message("Newsletter", "nothing to see")));
    }

    #[test]
    fn mega_mail_without_a_body_is_detected() {
        let empty = message(
            "MEGA email verification required",
            "<pre></pre>",
        );
        assert!(looks_like_mega(&empty));
        assert!(!is_confirmation(&empty));
    }
}
