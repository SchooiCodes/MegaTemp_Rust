use anyhow::{anyhow, bail, Result};
use serde_json::Value;
use std::time::Duration;

use super::{generate_address_local, Mailbox, Message};

const BASE: &str = "https://api.guerrillamail.com/ajax.php";

#[derive(Clone)]
pub struct Guerrilla {
    http: reqwest::Client,
}

impl Guerrilla {
    pub fn new() -> Result<Self> {
        Ok(Self {
            http: reqwest::Client::builder()
                .user_agent("MegaTemp")
                .timeout(Duration::from_secs(30))
                .build()?,
        })
    }

    async fn request(&self, params: &[(&str, String)]) -> Result<Value> {
        let mut last = None;
        for attempt in 1..=10u32 {
            match self.http.get(BASE).query(params).send().await {
                Ok(resp) => {
                    let status = resp.status();
                    let text = resp.text().await.unwrap_or_default();
                    if status.is_success() {
                        return Ok(serde_json::from_str(&text).unwrap_or(Value::Null));
                    }
                    last = Some(anyhow!("guerrillamail HTTP {status}: {text}"));
                    if status.is_server_error() || status.as_u16() == 429 {
                        tokio::time::sleep(Duration::from_millis(300 + attempt as u64 * 250)).await;
                        continue;
                    }
                    bail!("guerrillamail HTTP {status}: {text}");
                }
                Err(e) => {
                    last = Some(anyhow!("guerrillamail request failed: {e}"));
                    tokio::time::sleep(Duration::from_millis(300 + attempt as u64 * 250)).await;
                }
            }
        }
        Err(last.unwrap_or_else(|| anyhow!("guerrillamail request failed")))
    }

    pub async fn create(&self) -> Result<Mailbox> {
        let first = self
            .request(&[("f", "get_email_address".to_string())])
            .await?;
        let original = first
            .get("email_addr")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let domain = original
            .split_once('@')
            .map(|(_, d)| d.to_string())
            .unwrap_or_else(|| "guerrillamail.com".to_string());
        let local = generate_address_local();
        let sid = first
            .get("sid_token")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let resp = self
            .request(&[
                ("f", "set_email_user".to_string()),
                ("email_user", local.clone()),
                ("sid_token", sid),
            ])
            .await?;
        let address = resp
            .get("email_addr")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| format!("{local}@{domain}"));
        Ok(Mailbox {
            provider: "guerrillamail".to_string(),
            address,
            token: resp
                .get("sid_token")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            secret: String::new(),
        })
    }

    pub async fn login(&self, address: &str) -> Result<Mailbox> {
        let local = address.split('@').next().unwrap_or(address).to_string();
        let resp = self
            .request(&[
                ("f", "set_email_user".to_string()),
                ("email_user", local),
                ("sid_token", String::new()),
            ])
            .await?;
        Ok(Mailbox {
            provider: "guerrillamail".to_string(),
            address: resp
                .get("email_addr")
                .and_then(|v| v.as_str())
                .unwrap_or(address)
                .to_string(),
            token: resp
                .get("sid_token")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            secret: String::new(),
        })
    }

    pub async fn latest_message(&self, mailbox: &Mailbox) -> Result<Option<Message>> {
        let data = self
            .request(&[
                ("f", "check_email".to_string()),
                ("seq", "0".to_string()),
                ("sid_token", mailbox.token.clone()),
            ])
            .await?;
        let Some(list) = data.get("list").and_then(|v| v.as_array()) else {
            return Ok(None);
        };
        let Some(first) = list.first() else {
            return Ok(None);
        };
        let id = first
            .get("mail_id")
            .map(|v| v.to_string().trim_matches('"').to_string())
            .unwrap_or_default();
        if id.is_empty() {
            return Ok(None);
        }
        let detail = self
            .request(&[
                ("f", "fetch_email".to_string()),
                ("email_id", id.clone()),
                ("sid_token", mailbox.token.clone()),
            ])
            .await?;
        let get = |key: &str| {
            detail
                .get(key)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        };
        Ok(Some(Message {
            mail_id: id,
            mail_from: get("mail_from"),
            mail_subject: get("mail_subject"),
            mail_body: get("mail_body"),
        }))
    }
}
