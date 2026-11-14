use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};
use std::time::Duration;

use super::{generate_address_local, Mailbox, Message};
use crate::util::random_alnum;

const BASE: &str = "https://api.mail.tm";

#[derive(Clone)]
pub struct MailTm {
    http: reqwest::Client,
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
                    let text = resp.text().await.unwrap_or_default();
                    if status.is_success() {
                        return Ok(serde_json::from_str(&text).unwrap_or(Value::Null));
                    }
                    if status.as_u16() == 429 || status.is_server_error() {
                        let delay = (2u64.pow(attempt - 1) + rand::random::<u64>() % 2).min(10);
                        tokio::time::sleep(Duration::from_secs(delay)).await;
                        last_error = Some(anyhow!("mail.tm HTTP {status}: {text}"));
                        continue;
                    }
                    bail!("mail.tm HTTP {status}: {text}");
                }
                Err(e) => {
                    last_error = Some(anyhow!("mail.tm request failed: {e}"));
                    tokio::time::sleep(Duration::from_millis(
                        300 + (attempt as u64 % 5) * 250,
                    ))
                    .await;
                }
            }
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
        members
            .iter()
            .find_map(|m| m.get("domain").and_then(|d| d.as_str()))
            .map(str::to_string)
            .ok_or_else(|| anyhow!("mail.tm returned no usable domain"))
    }

    pub async fn create(&self) -> Result<Mailbox> {
        let domain = self.domain().await?;
        let address = format!("{}@{}", generate_address_local(), domain);
        let password = random_alnum(10);
        self.request(
            reqwest::Method::POST,
            &format!("{BASE}/accounts"),
            None,
            Some(json!({"address": address, "password": password})),
        )
        .await?;
        self.login(&address, &password).await
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
        let body = detail
            .get("text")
            .and_then(|v| v.as_str())
            .or_else(|| detail.get("html").and_then(|v| v.as_str()))
            .or_else(|| detail.get("intro").and_then(|v| v.as_str()))
            .unwrap_or("")
            .to_string();
        Ok(Some(Message {
            mail_id: id,
            mail_from: from,
            mail_subject: subject,
            mail_body: body,
        }))
    }
}
