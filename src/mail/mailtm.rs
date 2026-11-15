use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};
use std::time::Duration;

use super::{generate_address_local, Mailbox, Message, RateLimited};
use crate::util::random_alnum;

const BASE: &str = "https://api.mail.tm";

#[derive(Clone)]
pub struct MailTm {
    http: reqwest::Client,
}

/// mail.tm returns `html` as an array of parts, and may leave `text` empty.
fn text_of(value: &Value, key: &str) -> Option<String> {
    match value.get(key)? {
        Value::String(s) if !s.trim().is_empty() => Some(s.clone()),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|p| p.as_str())
            .find(|s| !s.trim().is_empty())
            .map(str::to_string),
        _ => None,
    }
}

impl MailTm {
    pub fn new() -> Result<Self> {
        Ok(Self {
            http: reqwest::Client::builder()
                .user_agent("MegaTemp")
                .timeout(Duration::from_secs(30))
                .build()?,
        })
    }

    async fn request(
        &self,
        method: reqwest::Method,
        url: &str,
        token: Option<&str>,
        body: Option<Value>,
    ) -> Result<Value> {
        let mut last_error = None;
        let mut rate_limited = false;
        // mail.tm applies a short per-IP window; the Python provider rides it out
        // over 10 attempts with exponential backoff, so do the same here.
        for attempt in 1..=10u32 {
            let mut req = self
                .http
                .request(method.clone(), url)
                .header("accept", "application/ld+json");
            if let Some(token) = token {
                req = req.bearer_auth(token);
            }
            if let Some(body) = &body {
                req = req.json(body);
            }
            match req.send().await {
                Ok(resp) => {
                    let status = resp.status();
                    let retry_after = resp
                        .headers()
                        .get("retry-after")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|v| v.parse::<u64>().ok());
                    let text = resp.text().await.unwrap_or_default();
                    if status.is_success() {
                        return Ok(serde_json::from_str(&text).unwrap_or(Value::Null));
                    }
                    let retryable = status.as_u16() == 429 || status.is_server_error();
                    if !retryable {
                        bail!("mail.tm HTTP {status}: {text}");
                    }
                    rate_limited = status.as_u16() == 429;
                    last_error = Some(anyhow!("mail.tm HTTP {status}: {text}"));
                    if attempt < 10 {
                        let delay = if status.as_u16() == 429 {
                            retry_after.map(|v| v.min(10)).unwrap_or_else(|| {
                                let base = 2f64.powi(attempt as i32 - 1);
                                (base + rand::random::<f64>()).min(10.0) as u64
                            })
                        } else {
                            (2u64.pow(attempt.min(6) - 1) + rand::random::<u64>() % 2).min(10)
                        };
                        crate::console::status_line(&format!(
                            "mail.tm busy ({status}), retrying in {delay}s"
                        ));
                        tokio::time::sleep(Duration::from_secs(delay)).await;
                    }
                }
                Err(e) => {
                    last_error = Some(anyhow!("mail.tm request failed: {e}"));
                    if attempt < 10 {
                        tokio::time::sleep(Duration::from_millis(
                            300 + (attempt as u64 % 5) * 250,
                        ))
                        .await;
                    }
                }
            }
        }
        crate::console::clear_status_line();
        if rate_limited {
            return Err(RateLimited {
                provider: "mail.tm",
                detail: last_error
                    .map(|e| e.to_string())
                    .unwrap_or_default()
                    .chars()
                    .take(120)
                    .collect(),
            }
            .into());
        }
        Err(last_error.unwrap_or_else(|| anyhow!("mail.tm request failed")))
    }

    async fn domain(&self) -> Result<String> {
        let data = self
            .request(reqwest::Method::GET, &format!("{BASE}/domains"), None, None)
            .await?;
        let members = data
            .get("hydra:member")
            .and_then(|v| v.as_array())
            .or_else(|| data.as_array())
            .ok_or_else(|| anyhow!("mail.tm returned no domains"))?;
        let active = members.iter().find(|m| {
            m.get("isActive").and_then(|v| v.as_bool()).unwrap_or(true)
                && m.get("domain")
                    .and_then(|d| d.as_str())
                    .map(|d| !d.is_empty())
                    .unwrap_or(false)
        });
        active
            .or_else(|| members.first())
            .and_then(|m| m.get("domain").and_then(|d| d.as_str()))
            .map(str::to_string)
            .ok_or_else(|| anyhow!("mail.tm returned no usable domain"))
    }

    pub async fn create(&self) -> Result<Mailbox> {
        let domain = self.domain().await?;
        let password = random_alnum(10);
        let mut last_error = None;
        // A rejected address (collision or validation) is retried with a new one,
        // mirroring the Python provider's `_unique_mail_address` loop.
        for attempt in 1..=10u32 {
            let address = format!("{}@{}", generate_address_local(), domain);
            match self
                .request(
                    reqwest::Method::POST,
                    &format!("{BASE}/accounts"),
                    None,
                    Some(json!({"address": address, "password": password})),
                )
                .await
            {
                Ok(created) => {
                    // mail.tm normalises the address it assigns (lowercase); logging in
                    // with the spelling we sent yields 401 Invalid credentials.
                    let assigned = created
                        .get("address")
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                        .unwrap_or(address);
                    return self.login(&assigned, &password).await;
                }
                Err(e) => {
                    if super::is_rate_limited(&e) {
                        return Err(e);
                    }
                    last_error = Some(e);
                    let delay = 0.3 + (attempt % 5) as f64 * 0.25;
                    tokio::time::sleep(Duration::from_secs_f64(delay)).await;
                }
            }
        }
        Err(last_error.unwrap_or_else(|| anyhow!("could not create a mail.tm account")))
    }

    pub async fn login(&self, address: &str, password: &str) -> Result<Mailbox> {
        let token_resp = self
            .request(
                reqwest::Method::POST,
                &format!("{BASE}/token"),
                None,
                Some(json!({"address": address, "password": password})),
            )
            .await?;
        let token = token_resp
            .get("token")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("mail.tm login returned no token"))?
            .to_string();
        Ok(Mailbox {
            provider: "mailtm".to_string(),
            address: address.to_string(),
            token,
            secret: password.to_string(),
        })
    }

    pub async fn latest_message(&self, mailbox: &Mailbox) -> Result<Option<Message>> {
        let data = self
            .request(
                reqwest::Method::GET,
                &format!("{BASE}/messages"),
                Some(&mailbox.token),
                None,
            )
            .await?;
        let members = data
            .get("hydra:member")
            .and_then(|v| v.as_array())
            .or_else(|| data.as_array());
        let Some(members) = members else {
            return Ok(None);
        };
        let Some(first) = members.first() else {
            return Ok(None);
        };
        let id = first
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if id.is_empty() {
            return Ok(None);
        }
        let detail = self
            .request(
                reqwest::Method::GET,
                &format!("{BASE}/messages/{id}"),
                Some(&mailbox.token),
                None,
            )
            .await?;
        let from = detail
            .get("from")
            .and_then(|f| f.get("address"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let subject = detail
            .get("subject")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let body = text_of(&detail, "text")
            .or_else(|| text_of(&detail, "html"))
            .or_else(|| text_of(&detail, "intro"))
            .unwrap_or_default();
        Ok(Some(Message {
            mail_id: id,
            mail_from: from,
            mail_subject: subject,
            mail_body: body,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_text_falls_back_to_html_parts() {
        let value = json!({
            "text": "",
            "html": ["<a href=\"https://mega.nz/#confirmXYZ\">confirm</a>"],
            "intro": "intro text"
        });
        assert_eq!(text_of(&value, "text"), None);
        assert!(text_of(&value, "html").unwrap().contains("mega.nz"));
        assert_eq!(text_of(&value, "intro").as_deref(), Some("intro text"));
    }
}
