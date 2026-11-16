use anyhow::Result;
use futures::stream::{self, StreamExt};
use serde::Serialize;
use std::io::IsTerminal;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::console::{p_print, separator, Color};
use crate::credentials::{self, LoadedCredential};
use crate::mega::{auth_failure, MegaClient};
use crate::models::{Config, Credentials};

#[derive(Debug, Clone, Serialize)]
pub struct AliveResult {
    pub email: String,
    pub age_days: f64,
    /// Days since the last successful login, for the human dashboard only.
    #[serde(skip_serializing)]
    pub last_login_days: Option<f64>,
    pub tags: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quota_gb: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct KeepaliveSummary {
    pub total: usize,
    pub alive: usize,
    pub dead: usize,
    pub errors: usize,
    pub pruned: usize,
}

fn now_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

fn backup_path(email: &str) -> PathBuf {
    let dir = credentials::dir().join("pruned");
    let _ = std::fs::create_dir_all(&dir);
    let stem = credentials::sanitize_filename(email);
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let mut candidate = dir.join(format!("{stem}.{stamp}.json"));
    let mut n = 1;
    while candidate.exists() {
        candidate = dir.join(format!("{stem}.{stamp}-{n}.json"));
        n += 1;
    }
    candidate
}

async fn login_with_retries(
    cfg: &Config,
    creds: &Credentials,
    proxy: Option<&str>,
) -> Result<MegaClient> {
    let mut last = None;
    for attempt in 1..=3 {
        let mut client = MegaClient::new(proxy)?;
        match client.login(&creds.email, &creds.password).await {
            Ok(()) => return Ok(client),
            Err(e) => {
                // A rejected password or removed account will never succeed on
                // retry, so stop burning attempts (and time) on it.
                let terminal = auth_failure(&e);
                last = Some(e);
                if terminal {
                    break;
                }
                let jitter = rand::random::<f64>() * 0.5 * attempt as f64;
                tokio::time::sleep(Duration::from_secs_f64(jitter)).await;
            }
        }
    }
    let _ = cfg;
    Err(last.unwrap_or_else(|| anyhow::anyhow!("login failed")))
}

pub async fn check_all(
    cfg: &Config,
    workers: usize,
    prune: bool,
    yes: bool,
    announce: bool,
) -> Result<(Vec<AliveResult>, KeepaliveSummary)> {
    let started = std::time::Instant::now();
    let accounts = credentials::load_all(&cfg.encryption_password, &cfg.account_format);
    if accounts.is_empty() {
        p_print("No saved accounts found.", Color::Warning);
        return Ok((Vec::new(), KeepaliveSummary::default()));
    }

    let mut allow_prune = prune;
    if prune && !yes && !std::io::stdin().is_terminal() {
        p_print(
            "Refusing to prune without --yes in a non-interactive session.",
            Color::Warning,
        );
        allow_prune = false;
    }

    let workers = if workers == 0 {
        accounts.len().min(4)
    } else {
        workers
    }
    .max(1);
    let total = accounts.len();
    let done = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let summary = Arc::new(tokio::sync::Mutex::new(KeepaliveSummary {
        total,
        ..Default::default()
    }));

    if announce {
        separator("Keep alive", 60);
    }
    let cfg_ref = cfg;
    let results: Vec<AliveResult> = stream::iter(accounts)
        .map(|account| {
            let done = Arc::clone(&done);
            let summary = Arc::clone(&summary);
            async move {
                let result = check_one(cfg_ref, &account, allow_prune, yes).await;
                let mut lock = summary.lock().await;
                match result.status.as_str() {
                    "alive" => lock.alive += 1,
                    "dead" | "pruned" => lock.dead += 1,
                    _ => lock.errors += 1,
                }
                lock.pruned += usize::from(result.status == "pruned");
                let current = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                if announce {
                    crate::console::status_line(&format!(
                        "[{current}/{total}] {}: {}",
                        result.email, result.status
                    ));
                }
                result
            }
        })
        .buffer_unordered(workers)
        .collect()
        .await;
    crate::console::clear_status_line();

    let summary = summary.lock().await.clone();
    if announce {
        separator("Keepalive summary", 60);
        p_print(
            &format!(
                "  {} accounts in {}",
                summary.total,
                crate::console::elapsed(started)
            ),
            Color::Cyan,
        );
        p_print(&format!("  Total:  {}", summary.total), Color::Blue);
        p_print(&format!("  Alive:  {}", summary.alive), Color::Ok);
        p_print(
            &format!("  Dead:   {}", summary.dead),
            if summary.dead == 0 { Color::Ok } else { Color::Warning },
        );
        if summary.errors > 0 {
            p_print(&format!("  Errors: {}", summary.errors), Color::Warning);
        }
        if summary.pruned > 0 {
            p_print(&format!("  Pruned: {}", summary.pruned), Color::Cyan);
        }
    }
    Ok((results, summary))
}

async fn check_one(cfg: &Config, account: &LoadedCredential, prune: bool, _yes: bool) -> AliveResult {
    let creds = &account.creds;
    let now = now_secs();
    // Python's health dashboard ages an account by its credential file mtime,
    // and reports last login separately; keep both.
    let age_days = if account.mtime > 0.0 {
        ((now - account.mtime) / 86400.0).max(0.0).round()
    } else {
        0.0
    };
    let last_login_days = if creds.last_login > 0.0 {
        Some((now - creds.last_login).max(0.0) / 86400.0)
    } else {
        None
    };
    let mut base = AliveResult {
        email: creds.email.clone(),
        age_days,
        last_login_days,
        tags: creds.tags.clone(),
        status: "error".to_string(),
        quota_gb: None,
        error: None,
    };

    if account.decrypt_failed {
        base.error = Some("could not decrypt credentials".to_string());
        return base;
    }

    let proxy = if cfg.proxy.is_empty() {
        None
    } else {
        Some(cfg.proxy.as_str())
    };

    match login_with_retries(cfg, creds, proxy).await {
        Ok(client) => match client.quota().await {
            Ok((_used, total)) => {
                base.status = "alive".to_string();
                let quota = ((total as f64 / 1_073_741_824.0) * 100.0).round() / 100.0;
                base.quota_gb = Some(quota);
                crate::console::v_print(&format!("    {quota:.2} GB left"), Color::Ok);
                let _ = credentials::touch_last_login(&creds.email, cfg, now_secs());
            }
            Err(e) => {
                base.error = Some(e.to_string());
            }
        },
        Err(err) => {
            let dead = auth_failure(&err);
            base.status = if dead { "dead".to_string() } else { "error".to_string() };
            base.error = Some(err.to_string());
            if dead && prune && !account.encrypted {
                let backup = backup_path(&creds.email);
                if std::fs::copy(&account.path, &backup).is_ok() {
                    if credentials::delete_credential(&creds.email, &cfg.account_format).is_ok() {
                        base.status = "pruned".to_string();
                    }
                }
            } else if dead && prune && account.encrypted {
                p_print(
                    &format!("Not pruning encrypted account {}", creds.email),
                    Color::Warning,
                );
            }
        }
    }
    base
}
