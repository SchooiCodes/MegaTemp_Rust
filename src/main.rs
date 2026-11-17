//! MegaTemp — bulk MEGA account creator and manager.
//!
//! This is a Rust port of the Python MegaTemp tool. Some helpers keep the
//! original module surface even when the CLI does not reach them yet, so the
//! crate opts out of dead-code warnings.

#![allow(dead_code)]

use megatemp::{
    browser, cli, config, console, credentials, disk, mail, menu, models, proxy, services, update,
    util,
};

use anyhow::{bail, Result};
use clap::Parser;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use browser::register::{register_with_retries, RegisterOptions};
use cli::Cli;
use console::{p_print, separator, Color};
use models::Credentials;

#[tokio::main]
async fn main() {
    let cli = Cli::parse_from(cli::normalize_args());
    if let Err(e) = run(cli).await {
        p_print(&format!("Error: {e:#}"), Color::Fail);
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
        disk::usage_report();
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
    console::set_verbose(cli.verbose);
    spawn_interrupt_handler();

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
    // Config quiet is additive, like Python's `--quiet or config.quiet`.
    if cfg.quiet {
        console::set_quiet(true);
    }

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
        if proxy::is_active() {
            // An explicit --proxy / --proxy-file wins, like Python.
        } else {
            match proxy::ProxyManager::fetch_from_url(url).await {
                Ok(list) if !list.is_empty() => {
                    let count = list.len();
                    let first = list[0].clone();
                    cfg.proxy = first.clone();
                    proxy::set_global(proxy::ProxyManager::from_list(list, cfg.proxy_per_attempt));
                    p_print(
                        &format!("Fetched {count} proxy(ies); using {first}"),
                        Color::Cyan,
                    );
                }
                Ok(_) => p_print("Proxy URL returned no proxies", Color::Warning),
                Err(e) => p_print(&format!("Could not fetch proxies: {e}"), Color::Warning),
            }
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
                        "error": r.error,
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
            println!("{}", serde_json::to_string_pretty(&payload)?);
        } else if !results.is_empty() {
            separator("Accounts", 60);
            for r in &results {
                let days = r.age_days as i64;
                let login = match r.last_login_days {
                    Some(d) if d >= 365.0 => ">1y ago".to_string(),
                    Some(d) => format!("{}d ago", d as i64),
                    None => "never".to_string(),
                };
                let tag = if r.tags.is_empty() {
                    String::new()
                } else {
                    format!(" [{}]", r.tags)
                };
                if r.status == "alive" {
                    let quota = r.quota_gb.unwrap_or(0.0);
                    p_print(
                        &format!(
                            "  OK   {:<35} {:>5.1} GB {} {:>3}d old ll:{}{}",
                            r.email,
                            quota,
                            util::storage_bar(quota, 20),
                            days,
                            login,
                            tag
                        ),
                        Color::Ok,
                    );
                } else {
                    let label = match r.status.as_str() {
                        "pruned" => "PRUNED",
                        "error" => "ERROR",
                        _ => "DEAD",
                    };
                    p_print(
                        &format!(
                            "  {:<5} {:<35} {:>8}   {:>3}d old ll:{}{}",
                            label, r.email, "-", days, login, tag
                        ),
                        Color::Warning,
                    );
                }
                if let Some(err) = &r.error {
                    p_print(&format!("      {err}"), Color::Warning);
                }
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
        let accounts = credentials::load_all(&cfg.encryption_password, &cfg.account_format);
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

    if let Some(count) = cli.internal_batch {
        run_internal_batch(&options, count).await;
        return Ok(());
    }

    if let Some(total) = cli.loop_count {
        if cli.parallel > 1 {
            run_parallel(
                &options,
                total,
                cli.parallel as usize,
                cli.resume,
                cli.isolate,
                &browser,
                &cli,
            )
            .await?;
        } else {
            run_loop(&options, total, cli.resume, cli.interval).await?;
        }
        return Ok(());
    }

    if cli.parallel > 1 {
        run_parallel(
            &options,
            cli.parallel as u64,
            cli.parallel as usize,
            false,
            cli.isolate,
            &browser,
            &cli,
        )
        .await?;
        return Ok(());
    }

    let single = cli.file.is_some()
        || cli.upload_dir.is_some()
        || cli.visible
        || cli.attempts != 4
        || cli.export_csv
        || cli.export_jsonl
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
        p_print(&format!("Registration {}/{total}", i + 1), Color::Header);
        match register_with_retries(options).await {
            Ok(account) => {
                state.completed += 1;
                p_print(&format!("  Created {}", account.email), Color::Ok);
            }
            Err(e) => {
                state.failed += 1;
                p_print(&format!("  Failed: {e:#}"), Color::Fail);
            }
        }
        let done = state.completed + state.failed;
        p_print(
            &format!(
                "[{done}/{total}] ok={} failed={} ({})",
                state.completed,
                state.failed,
                console::elapsed(started)
            ),
            Color::Blue,
        );
        if (state.completed + state.failed) % 5 == 0 || last_save.elapsed() > Duration::from_secs(30)
        {
            let _ = console::save_checkpoint(&state);
            last_save = Instant::now();
        }
    }
    let _ = console::save_checkpoint(&state);
    let elapsed = console::elapsed(started);
    separator("Loop summary", 60);
    p_print(&format!("  Total accounts: {total}"), Color::Blue);
    p_print(&format!("  Successful:     {}", state.completed), Color::Ok);
    p_print(
        &format!("  Failed:         {}", state.failed),
        if state.failed == 0 { Color::Ok } else { Color::Warning },
    );
    p_print(&format!("  Total time:     {elapsed}"), Color::Cyan);
    if state.completed > 0 {
        p_print(
            &format!(
                "  Avg / success:  {:.1}s",
                started.elapsed().as_secs_f64() / state.completed as f64
            ),
            Color::Cyan,
        );
    }
    p_print("Done.", Color::Ok);
    console::notify(
        "MegaTemp: Loop complete",
        &format!("{} OK / {} FAIL in {elapsed}", state.completed, state.failed),
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn run_parallel(
    options: &RegisterOptions,
    total: u64,
    workers: usize,
    resume: bool,
    isolate: bool,
    browser: &Path,
    cli: &Cli,
) -> Result<()> {
    let total = total.max(1);
    let workers = workers.max(1).min(total as usize);
    let started = Instant::now();
    let skip = if resume {
        console::load_checkpoint()
            .map(|state| state.completed + state.failed)
            .unwrap_or(0)
            .min(total)
    } else {
        0
    };
    let remaining = total - skip;
    if remaining == 0 {
        p_print("Nothing left to do from the checkpoint", Color::Warning);
        return Ok(());
    }
    p_print(
        &format!("Creating {remaining} account(s) with {workers} worker(s)"),
        Color::Cyan,
    );

    let (ok, failed) = if isolate {
        run_isolated_workers(remaining, workers, browser, cli).await?
    } else {
        run_shared_workers(options, remaining, workers, skip, total, started).await
    };

    let elapsed = console::elapsed(started);
    separator("Parallel summary", 60);
    p_print(&format!("  Total accounts: {total}"), Color::Blue);
    if skip > 0 {
        p_print(&format!("  Resumed at:     {skip}"), Color::Cyan);
    }
    p_print(&format!("  Successful:     {ok}"), Color::Ok);
    p_print(
        &format!("  Failed:         {failed}"),
        if failed == 0 {
            Color::Ok
        } else {
            Color::Warning
        },
    );
    p_print(&format!("  Total time:     {elapsed}"), Color::Cyan);
    if ok > 0 {
        p_print(
            &format!(
                "  Avg / success:  {:.1}s",
                started.elapsed().as_secs_f64() / ok as f64
            ),
            Color::Cyan,
        );
    }
    p_print("Done.", Color::Ok);
    console::notify(
        "MegaTemp: Parallel complete",
        &format!("{ok} OK / {failed} FAIL in {elapsed}"),
    );
    Ok(())
}

/// In-process workers that claim account slots from one shared counter.
async fn run_shared_workers(
    options: &RegisterOptions,
    remaining: u64,
    workers: usize,
    skip: u64,
    total: u64,
    started: Instant,
) -> (u64, u64) {
    let next = Arc::new(AtomicU64::new(0));
    let ok_counter = Arc::new(AtomicU64::new(0));
    let fail_counter = Arc::new(AtomicU64::new(0));
    let mut tasks = tokio::task::JoinSet::new();

    for (worker, slot) in proxy::distribute(workers).into_iter().enumerate() {
        let mut worker_options = options.clone();
        if slot.is_some() {
            worker_options.proxy = slot;
        }
        let next = next.clone();
        let ok_counter = ok_counter.clone();
        let fail_counter = fail_counter.clone();
        let label = format!("w{}", worker + 1);
        tasks.spawn(console::labelled(label, async move {
            loop {
                let index = next.fetch_add(1, Ordering::Relaxed);
                if index >= remaining {
                    break;
                }
                match console::collected(register_with_retries(&worker_options)).await {
                    Ok(account) => {
                        ok_counter.fetch_add(1, Ordering::Relaxed);
                        p_print(&format!("Created {}", account.email), Color::Ok);
                    }
                    Err(e) => {
                        fail_counter.fetch_add(1, Ordering::Relaxed);
                        p_print(&format!("Failed: {e:#}"), Color::Fail);
                    }
                }
                let ok = ok_counter.load(Ordering::Relaxed);
                let failed = fail_counter.load(Ordering::Relaxed);
                p_print(
                    &format!(
                        "[{}/{total}] ok={ok} failed={failed} ({})",
                        skip + ok + failed,
                        console::elapsed(started)
                    ),
                    Color::Blue,
                );
            }
        }));
    }

    while tasks.join_next().await.is_some() {}
    (
        ok_counter.load(Ordering::Relaxed),
        fail_counter.load(Ordering::Relaxed),
    )
}

/// One child process per worker. Each child registers its batch sequentially
/// and exits with the number of failures, so the parent can report totals.
async fn run_isolated_workers(
    remaining: u64,
    workers: usize,
    browser: &Path,
    cli: &Cli,
) -> Result<(u64, u64)> {
    let exe = std::env::current_exe()?;
    let per_worker = remaining.div_ceil(workers as u64);
    let mut slots = proxy::distribute(workers).into_iter();
    let mut tasks = tokio::task::JoinSet::new();
    let mut assigned = 0u64;

    while assigned < remaining {
        let batch = per_worker.min(remaining - assigned);
        let slot = slots.next().flatten();
        let mut command = tokio::process::Command::new(&exe);
        command
            .arg("--internal-batch")
            .arg(batch.to_string())
            .arg("--attempts")
            .arg(cli.attempts.to_string())
            .arg("--browser")
            .arg(browser);
        if let Some(provider) = &cli.provider {
            command.arg("--provider").arg(provider);
        }
        if let Some(profile) = &cli.profile {
            command.arg("--profile").arg(profile);
        }
        if let Some(timeout) = cli.mail_timeout {
            command.arg("--mail-timeout").arg(timeout.to_string());
        }
        if let Some(password) = &cli.encryption_password {
            command.arg("--encryption-password").arg(password);
        }
        if let Some(proxy) = slot {
            command.arg("--proxy").arg(proxy);
        }
        for (enabled, flag) in [
            (cli.visible, "--visible"),
            (cli.quiet, "--quiet"),
            (cli.keep_profiles, "--keep-profiles"),
            (cli.export_csv, "--export-csv"),
            (cli.export_jsonl, "--export-jsonl"),
            (cli.verbose, "--verbose"),
        ] {
            if enabled {
                command.arg(flag);
            }
        }
        command.stdin(std::process::Stdio::null());

        match command.spawn() {
            Ok(mut child) => {
                tasks.spawn(async move {
                    let failed = match child.wait().await {
                        Ok(status) => status
                            .code()
                            .map(|code| (code.max(0) as u64).min(batch))
                            .unwrap_or(batch),
                        Err(_) => batch,
                    };
                    (batch - failed, failed)
                });
            }
            Err(e) => {
                p_print(&format!("Failed to start worker: {e}"), Color::Fail);
                tasks.spawn(async move { (0u64, batch) });
            }
        }
        assigned += batch;
    }

    let mut ok = 0u64;
    let mut failed = 0u64;
    while let Some(joined) = tasks.join_next().await {
        if let Ok((worker_ok, worker_failed)) = joined {
            ok += worker_ok;
            failed += worker_failed;
        }
        p_print(
            &format!("[{}/{remaining}] ok={ok} failed={failed}", ok + failed),
            Color::Blue,
        );
    }
    Ok((ok, failed))
}

/// Child-process body for `--isolate`. Registers `count` accounts and exits
/// with the failure count so the parent process can total the run.
async fn run_internal_batch(options: &RegisterOptions, count: u64) {
    let mut failed = 0u64;
    for _ in 0..count {
        match register_with_retries(options).await {
            Ok(account) => p_print(&format!("Created {}", account.email), Color::Ok),
            Err(e) => {
                failed += 1;
                p_print(&format!("Failed: {e:#}"), Color::Fail);
            }
        }
    }
    std::process::exit(failed.min(255) as i32);
}

/// Print the final totals even when the process is interrupted (Ctrl+C).
fn spawn_interrupt_handler() {
    tokio::spawn(async {
        if tokio::signal::ctrl_c().await.is_ok() {
            use std::io::Write;
            let mut out = std::io::stdout();
            let _ = crossterm::execute!(
                out,
                crossterm::cursor::Show,
                crossterm::terminal::LeaveAlternateScreen
            );
            let _ = crossterm::terminal::disable_raw_mode();
            let _ = writeln!(out);
            p_print("Interrupted. Exiting...", Color::Warning);
            std::process::exit(130);
        }
    });
}

fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

#[allow(dead_code)]
fn _unused(_: &Path, _: &PathBuf) {}
