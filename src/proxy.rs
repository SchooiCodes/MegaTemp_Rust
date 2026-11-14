use crate::console::{p_print, Color};
use anyhow::{bail, Result};
use std::sync::atomic::{AtomicUsize, Ordering};

pub fn validate(proxy: &str) -> Result<()> {
    if proxy.is_empty() {
        return Ok(());
    }
    let (scheme, rest) = proxy
        .split_once("://")
        .ok_or_else(|| anyhow::anyhow!("proxy is missing a scheme"))?;
    if !["http", "https", "socks4", "socks5"].contains(&scheme) {
        bail!("unsupported proxy scheme '{scheme}'");
    }
    if rest.matches('@').count() > 1 {
        bail!("proxy has more than one '@'");
    }
    let host = rest.rsplit('@').next().unwrap_or(rest);
    if host.split(':').next().unwrap_or("").is_empty() {
        bail!("proxy has no hostname");
    }
    Ok(())
}

#[derive(Debug, Default)]
pub struct ProxyManager {
    proxies: Vec<String>,
    per_attempt: bool,
    index: AtomicUsize,
}

impl ProxyManager {
    pub fn new(proxy: &str, proxy_file: &str, per_attempt: bool) -> Self {
        let mut proxies = Vec::new();
        if !proxy_file.is_empty() {
            if !proxy.is_empty() {
                p_print(
                    "Warning: both proxy and proxy file configured; using the file",
                    Color::Warning,
                );
            }
            match std::fs::read_to_string(&proxy_file) {
                Ok(text) => {
                    for line in text.lines() {
                        let line = line.trim();
                        if line.is_empty() || line.starts_with('#') {
                            continue;
                        }
                        proxies.push(line.to_string());
                    }
                }
                Err(e) => p_print(
                    &format!("Warning: could not read proxy file {proxy_file}: {e}"),
                    Color::Warning,
                ),
            }
        } else if !proxy.is_empty() {
            proxies.push(proxy.to_string());
        }
        Self {
            proxies,
            per_attempt,
            index: AtomicUsize::new(0),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.proxies.is_empty()
    }

    pub fn get_proxy(&self) -> Option<String> {
        if self.proxies.is_empty() {
            return None;
        }
        if self.per_attempt || self.proxies.len() == 1 {
            let i = self.index.fetch_add(1, Ordering::Relaxed) % self.proxies.len();
            Some(self.proxies[i].clone())
        } else {
            Some(self.proxies[0].clone())
        }
    }

    /// Round-robin distribution for N workers.
    pub fn distribute(&self, n: usize) -> Vec<Option<String>> {
        (0..n)
            .map(|_| {
                if self.proxies.is_empty() {
                    None
                } else {
                    let i = self.index.fetch_add(1, Ordering::Relaxed) % self.proxies.len();
                    Some(self.proxies[i].clone())
                }
            })
            .collect()
    }

    pub async fn fetch_from_url(url: &str) -> Result<Vec<String>> {
        let client = reqwest::Client::builder()
            .user_agent("MegaTemp")
            .timeout(std::time::Duration::from_secs(15))
            .build()?;
        let resp = client.get(url).send().await?.error_for_status()?;
        let text = resp.text().await?;
        if let Ok(list) = serde_json::from_str::<Vec<String>>(&text) {
            return Ok(list.into_iter().filter(|s| !s.trim().is_empty()).collect());
        }
        Ok(text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(str::to_string)
            .collect())
    }

    pub async fn test_proxy(proxy: &str) -> Result<String> {
        validate(proxy)?;
        let mut builder = reqwest::Client::builder().timeout(std::time::Duration::from_secs(20));
        if !proxy.is_empty() {
            builder = builder.proxy(reqwest::Proxy::all(proxy)?);
        }
        let client = builder.build()?;
        let resp = client.get("https://httpbin.org/ip").send().await?;
        Ok(resp.text().await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_valid_schemes() {
        for p in ["http://1.2.3.4:8080", "socks5://user:pass@host:1080"] {
            assert!(validate(p).is_ok(), "{p}");
        }
        assert!(validate("1.2.3.4:8080").is_err());
        assert!(validate("ftp://host:21").is_err());
    }

    #[test]
    fn rotation_modes() {
        let pm = ProxyManager::new("http://a:1", "", false);
        assert_eq!(pm.get_proxy().unwrap(), "http://a:1");
        let pm = ProxyManager {
            proxies: vec!["a".into(), "b".into()],
            per_attempt: false,
            index: AtomicUsize::new(0),
        };
        assert_eq!(pm.get_proxy().unwrap(), "a");
        assert_eq!(pm.get_proxy().unwrap(), "a");
        let pm = ProxyManager {
            proxies: vec!["a".into(), "b".into()],
            per_attempt: true,
            index: AtomicUsize::new(0),
        };
        assert_eq!(pm.get_proxy().unwrap(), "a");
        assert_eq!(pm.get_proxy().unwrap(), "b");
    }
}
