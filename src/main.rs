//! MegaTemp — bulk MEGA account creator and manager.
//!
//! This is a Rust port of the Python MegaTemp tool. Some helpers keep the
//! original module surface even when the CLI does not reach them yet, so the
//! crate opts out of dead-code warnings.

#![allow(dead_code)]

mod browser;
mod cli;
mod config;
mod console;
mod credentials;
mod crypto;
mod disk;
mod hashcash;
mod lock;
mod mail;
mod mega;
mod menu;
mod models;
mod password;
mod proxy;
mod services;
mod update;
mod util;

use anyhow::{bail, Result};
use clap::Parser;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use browser::register::{register_with_retries, RegisterOptions};
use cli::Cli;
use console::{p_print, separator, Color};
use models::Credentials;

#[tokio::main]
async fn main() {
    let cli = Cli::parse_from(cli::normalize_args());
    if let Err(e) = run(cli).await {
        p_print(&format!("Error: {e}"), Color::Fail);
        std::process::exit(1);
    }
}

async fn run(cli: Cli) -> Result<()> {
    if cli.json && !cli.health {
        eprintln!("--json can only be used together with --health");
        std::process::exit(2);
    }

    if cli.clean {
        disk::clean_all();
        return Ok(());
    }
    if cli.disk_usage {
        disk::usage_report();
        return Ok(());
    }

    let _ = disk::cleanup_stale_profiles(Duration::from_secs(24 * 60 * 60));
    let _ = disk::prune_backups(disk::DEFAULT_BACKUP_KEEP);

    console::set_quiet(cli.quiet);
    console::set_json(cli.json);
    console::set_notify(cli.notify);

    if let Some(provider) = &cli.provider {
        if !mail::valid_providers().contains(&provider.to_lowercase().as_str()) {
            eprintln!(
                "Unknown provider '{provider}'. Valid providers: {}",
                mail::valid_providers().join(", ")
            );
            std::process::exit(2);
        }
    }

    let mut cfg = config::load(cli.profile.as_deref());

    // CLI overrides take precedence over the config file.
    if let Some(provider) = &cli.provider {
        cfg.email_provider = provider.clone();
    }
    if !mail::valid_providers().contains(&cfg.email_provider.to_lowercase().as_str()) {
        eprintln!(
            "Unknown provider '{}'. Valid providers: {}",
            cfg.email_provider,
            mail::valid_providers().join(", ")
        );
        std::process::exit(2);
    }
    cfg.max_attempts = cli.attempts;
    if let Some(timeout) = cli.mail_timeout {
        cfg.mail_timeout = timeout;
    }
    if let Some(proxy) = &cli.proxy {
        cfg.proxy = proxy.clone();
    }
    if let Some(file) = &cli.proxy_file {
        cfg.proxy_file = file.clone();
    }
    if cli.proxy_per_attempt {
        cfg.proxy_per_attempt = true;
    }
    if cli.visible {
        cfg.visible_browser = true;
    }
    if cli.quiet {
        cfg.quiet = true;
    }
    if let Some(password) = &cli.encryption_password {
        cfg.encryption_password = password.clone();
    }
    if let Some(url) = &cli.webhook_url {
        cfg.webhook_url = url.clone();
    }
    if cli.export_csv {
        cfg.csv_export = true;
    }
    if cli.export_jsonl {
        cfg.jsonl_export = true;
    }
    // Global proxy manager: mirrors the Python `_proxy_manager` rotation.
    proxy::set_global(proxy::ProxyManager::new(
        &cfg.proxy,
        &cfg.proxy_file,
        cfg.proxy_per_attempt,
    ));

    if let Some(url) = &cli.proxy_url {
        match proxy::ProxyManager::fetch_from_url(url).await {
            Ok(list) if !list.is_empty() => {
                cfg.proxy = list[0].clone();
                p_print(&format!("Using fetched proxy {}", cfg.proxy), Color::Cyan);
            }
            Ok(_) => p_print("Proxy URL returned no proxies", Color::Warning),
            Err(e) => p_print(&format!("Could not fetch proxies: {e}"), Color::Warning),
        }
    }

    if cli.update {
        update::run_update(cli.yes).await?;
        return Ok(());
    }

    if cli.health {
        let (results, summary) =
            services::alive::check_all(&cfg, cli.workers, false, true, !cli.json).await?;
        if cli.json {
            let accounts: Vec<serde_json::Value> = results
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "email": r.email,
                        "age_days": r.age_days,
                        "tags": r.tags,
                        "status": r.status,
                        "quota_gb": r.quota_gb,
                    })
                })
                .collect();
            let payload = serde_json::json!({
                "accounts": accounts,
                "summary": {
                    "alive": summary.alive,
                    "dead": summary.dead,
                    "total": summary.total
                }
            });
            println!("{}", serde_json::to_string(&payload)?);
        } else if !results.is_empty() {
            separator("Accounts", 60);
            for r in &results {
                println!(
                    "  {:<34} {:<8} {:>8}  {}",
                    r.email,
                    r.status,
                    r.quota_gb
                        .map(|q| format!("{q:.1} GB"))
                        .unwrap_or_else(|| "-".to_string()),
                    r.tags
                );
            }
        }
        return Ok(());
    }

    if cli.extract {
        services::extract::extract_credentials(&cfg, &cfg.account_format)?;
        return Ok(());
    }
    if cli.export_keepass {
        services::extract::export_keepass(&cfg)?;
    }
    if cli.export_bitwarden {
        services::extract::export_bitwarden(&cfg)?;
    }
    if cli.export_onepassword {
        services::extract::export_onepassword(&cfg)?;
    }
    if cli.export_keepass || cli.export_bitwarden || cli.export_onepassword {
        return Ok(());
    }

    if cli.list_cloud || cli.download_cloud.is_some() {
        let accounts = credentials::load_all(&cfg.encryption_password);
        let Some(account) = accounts.into_iter().find(|a| !a.decrypt_failed) else {
            bail!("no usable saved accounts found");
        };
        if cli.list_cloud {
            let files = services::download::list_files(&cfg, &account.creds).await?;
            separator("Cloud files", 60);
            for file in &files {
                println!(
                    "  {:<14} {:<42} {:>10}",
                    file.handle,
                    file.name,
                    util::human_size(file.size)
                );
            }
        }
        if let Some(id) = &cli.download_cloud {
            let files = services::download::list_files(&cfg, &account.creds).await?;
            let node = files
                .iter()
                .find(|n| &n.handle == id || &n.name == id)
                .ok_or_else(|| anyhow::anyhow!("cloud file '{id}' not found"))?;
            services::download::download_file(&cfg, &account.creds, node, &cli.download_dest)
                .await?;
        }
        return Ok(());
    }

    if cli.keepalive {
        loop {
            services::alive::check_all(&cfg, cli.workers, cli.prune, cli.yes, true).await?;
            let Some(hours) = cli.interval else {
                break;
            };
            p_print(
                &format!("Sleeping for {hours} hour(s) before the next pass"),
                Color::Cyan,
            );
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs_f64(hours.max(0.01) * 3600.0)) => {}
                _ = tokio::signal::ctrl_c() => break,
            }
        }
        return Ok(());
    }

    // Anything below this point needs a browser.
    let browser = match browser::register::resolve_browser(&cfg, cli.browser.as_deref()) {
        Some(path) => path,
        None => {
            if !std::io::stdin().is_terminal() {
                eprintln!("No browser found. Install Chrome/Chromium or pass --browser.");
                std::process::exit(2);
            }
            let entered = util::prompt(
                "No browser found. Enter the path to Chrome/Chromium/Edge: ",
            );
            let path = PathBuf::from(util::expand_user(entered.trim()));
            if !path.is_file() {
                eprintln!("Browser not found at {}", path.display());
                std::process::exit(2);
            }
            cfg.executable_path = path.to_string_lossy().to_string();
            let _ = config::save(cli.profile.as_deref(), &cfg);
            path
        }
    };

    let options = RegisterOptions {
        cfg: cfg.clone(),
        provider: cfg.email_provider.clone(),
        proxy: if cfg.proxy.is_empty() {
            None
        } else {
            Some(cfg.proxy.clone())
        },
        visible: cfg.visible_browser,
        browser: browser.clone(),
        keep_profile: cli.keep_profiles,
    };

    if let Some(total) = cli.loop_count {
        run_loop(&options, total, cli.resume, cli.interval).await?;
        return Ok(());
    }

    if cli.parallel > 1 {
        run_parallel(&options, cli.parallel as u64, cli.isolate).await?;
        return Ok(());
    }

    let single = cli.file.is_some()
        || cli.upload_dir.is_some()
        || cli.visible
        || cli.attempts != 4
        || cli.export_csv
        || cli.verbose;

    if single {
        let account = register_with_retries(&options).await?;
        if let Some(file) = &cli.file {
            let creds = Credentials::new(account.email.clone(), account.password.clone());
            services::upload::upload_file(&cfg, &creds, file, cli.public).await?;
        }
        if let Some(dir) = &cli.upload_dir {
            let creds = Credentials::new(account.email.clone(), account.password.clone());
            services::upload::upload_dir(&cfg, &creds, dir, cli.public).await?;
        }
        return Ok(());
    }

    update::auto_update().await;
    let mut menu = menu::Menu {
        cfg,
        profile: cli.profile.clone(),
        browser,
    };
    menu.run().await
}

async fn run_loop(
    options: &RegisterOptions,
    total: u64,
    resume: bool,
    _interval: Option<f64>,
) -> Result<()> {
    let total = total.max(1);
    let mut state = if resume {
        console::load_checkpoint().unwrap_or(console::LoopState {
            total,
            started_at: now(),
            ..Default::default()
        })
    } else {
        console::LoopState {
            total,
            started_at: now(),
            ..Default::default()
        }
    };
    state.total = total;
    let started = Instant::now();
    let mut last_save = Instant::now();

    for i in 0..total {
        if resume && i < state.completed + state.failed {
            continue;
        }
        p_print(&format!("Registration {}/{total}", i + 1), Color::Cyan);
        match register_with_retries(options).await {
            Ok(account) => {
                state.completed += 1;
                p_print(&format!("Created {}", account.email), Color::Ok);
            }
            Err(e) => {
                state.failed += 1;
                p_print(&format!("Failed: {e}"), Color::Fail);
            }
        }
        if (state.completed + state.failed) % 5 == 0 || last_save.elapsed() > Duration::from_secs(30)
        {
            let _ = console::save_checkpoint(&state);
            last_save = Instant::now();
        }
    }
    let _ = console::save_checkpoint(&state);
    let total_done = state.completed + state.failed;
    separator("Summary", 60);
    println!(
        "  Total: {total_done}  Success: {}  Failed: {}  Time: {}",
        state.completed,
        state.failed,
        console::elapsed(started)
    );
    if total_done > 0 {
        println!("  Average: {:.1}s/account", started.elapsed().as_secs_f64() / total_done as f64);
    }
    console::notify("MegaTemp", &format!("{} account(s) created", state.completed));
    Ok(())
}

async fn run_parallel(options: &RegisterOptions, total: u64, _isolate: bool) -> Result<()> {
    let total = total.max(1);
    p_print(&format!("Creating {total} account(s) in parallel"), Color::Cyan);
    let slots = proxy::distribute(total as usize);
    let mut handles = Vec::new();
    for (worker, slot) in slots.into_iter().enumerate() {
        let mut options = options.clone();
        if slot.is_some() {
            options.proxy = slot;
        }
        handles.push(tokio::spawn(async move {
            let result = register_with_retries(&options).await;
            let ok = result.is_ok();
            match result {
                Ok(account) => println!("[w{}] created {}", worker + 1, account.email),
                Err(e) => println!("[w{}] failed: {e}", worker + 1),
            }
            ok
        }));
    }
    let mut ok = 0;
    let mut failed = 0;
    for handle in handles {
        match handle.await {
            Ok(true) => ok += 1,
            Ok(false) => failed += 1,
            Err(_) => failed += 1,
        }
    }
    separator("Summary", 60);
    println!("  Total: {total}  Success: {ok}  Failed: {failed}");
    Ok(())
}

fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

#[allow(dead_code)]
fn _unused(_: &Path, _: &PathBuf) {}
