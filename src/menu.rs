use anyhow::{anyhow, Result};
use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{read, Event, KeyCode, KeyEventKind};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, Clear, ClearType};
use crossterm::execute;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::browser::register::{register_with_retries, RegisterOptions};
use crate::console::{p_print, separator, Color};
use crate::credentials;
use crate::models::{Config, Credentials};
use crate::password;
use crate::services;
use crate::util::{prompt, prompt_yes_no};

pub struct Menu {
    pub cfg: Config,
    pub profile: Option<String>,
    pub browser: PathBuf,
}

impl Menu {
    pub async fn run(&mut self) -> Result<()> {
        loop {
            let items = [
                "Create Accounts",
                "Credentials",
                "Account Management",
                "Upload & Cloud",
                "Settings",
                "Exit",
            ];
            let Some(choice) = select("MegaTemp", &items)? else {
                return Ok(());
            };
            match choice {
                0 => self.create_accounts().await?,
                1 => self.credentials_menu()?,
                2 => self.account_management().await?,
                3 => self.cloud_menu().await?,
                4 => self.settings().await?,
                _ => {
                    if prompt_yes_no("Exit MegaTemp?", true) {
                        return Ok(());
                    }
                }
            }
        }
    }

    async fn create_accounts(&mut self) -> Result<()> {
        let items = ["Single account", "Batch (sequential)", "Batch (parallel)", "Back"];
        let Some(choice) = select("Create Accounts", &items)? else {
            return Ok(());
        };
        let (accounts, parallel) = match choice {
            0 => (1u64, 1u32),
            1 => {
                let n = prompt("How many accounts? [1]: ");
                (n.parse().unwrap_or(1), 1)
            }
            2 => {
                let n = prompt("How many accounts? [4]: ");
                let count = n.parse().unwrap_or(4);
                let workers = prompt("Parallel workers? [4]: ");
                (count, workers.parse().unwrap_or(4))
            }
            _ => return Ok(()),
        };
        self.register_batch(accounts, parallel).await
    }

    async fn register_batch(&mut self, total: u64, parallel: u32) -> Result<()> {
        let options = RegisterOptions {
            cfg: self.cfg.clone(),
            provider: self.cfg.email_provider.clone(),
            proxy: if self.cfg.proxy.is_empty() {
                None
            } else {
                Some(self.cfg.proxy.clone())
            },
            visible: self.cfg.visible_browser,
            browser: self.browser.clone(),
            keep_profile: false,
        };
        if parallel <= 1 {
            for i in 1..=total {
                println!("Registration {i}/{total}");
                if let Err(e) = register_with_retries(&options).await {
                    p_print(&format!("Registration failed: {e}"), Color::Fail);
                }
            }
        } else {
            let mut handles = Vec::new();
            for worker in 0..parallel {
                let options = options.clone();
                handles.push(tokio::spawn(async move {
                    for i in 0..total.div_ceil(parallel as u64) {
                        if i > 0 {
                            tokio::time::sleep(Duration::from_secs(1)).await;
                        }
                        match register_with_retries(&options).await {
                            Ok(a) => println!("[w{}] created {}", worker + 1, a.email),
                            Err(e) => println!("[w{}] failed: {e}", worker + 1),
                        }
                    }
                }));
            }
            for handle in handles {
                let _ = handle.await;
            }
        }
        Ok(())
    }

    fn credentials_menu(&mut self) -> Result<()> {
        loop {
            let items = [
                "View accounts",
                "Export credentials.txt",
                "Export Bitwarden CSV",
                "Export 1Password CSV",
                "Export KeePass CSV",
                "Back",
            ];
            let Some(choice) = select("Credentials", &items)? else {
                return Ok(());
            };
            match choice {
                0 => self.view_accounts()?,
                1 => {
                    services::extract::extract_credentials(&self.cfg, &self.cfg.account_format)?;
                }
                2 => {
                    services::extract::export_bitwarden(&self.cfg)?;
                }
                3 => {
                    services::extract::export_onepassword(&self.cfg)?;
                }
                4 => {
                    services::extract::export_keepass(&self.cfg)?;
                }
                _ => return Ok(()),
            }
        }
    }

    fn view_accounts(&mut self) -> Result<()> {
        let mut accounts = credentials::load_all(&self.cfg.encryption_password);
        if accounts.is_empty() {
            println!("No saved accounts.");
            return Ok(());
        }
        for (i, account) in accounts.iter().enumerate() {
            let masked = mask(&account.creds.password);
            println!(
                "{:>3}. {}  {}  {}",
                i + 1,
                account.creds.email,
                masked,
                if account.decrypt_failed { "(locked)" } else { "" }
            );
        }
        let choice = prompt("Account number (blank to go back): ");
        let Ok(index) = choice.parse::<usize>() else {
            return Ok(());
        };
        if index == 0 || index > accounts.len() {
            return Ok(());
        }
        let mut account = accounts.remove(index - 1);
        loop {
            separator(&account.creds.email, 60);
            println!("Email:          {}", account.creds.email);
            println!("Email password: {}", mask(&account.creds.email_password));
            println!("MEGA password:  {}", mask(&account.creds.password));
            let (bits, label, _) = password::strength(&account.creds.password);
            println!("Strength:       {label} ({bits:.1} bits)");
            println!("Tags:           {}", account.creds.tags);
            println!("Notes:          {}", account.creds.notes);
            println!(
                "Last login:     {}",
                if account.creds.last_login > 0.0 {
                    chrono::DateTime::from_timestamp(account.creds.last_login as i64, 0)
                        .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
                        .unwrap_or_else(|| "-".to_string())
                } else {
                    "-".to_string()
                }
            );
            let action = prompt("[r]eveal  [n]otes  [t]ags  [d]elete  [b]ack: ");
            match action.trim().to_lowercase().as_str() {
                "r" => {
                    println!("MEGA password:  {}", account.creds.password);
                }
                "n" => {
                    account.creds.notes = prompt("Notes: ");
                    let _ = credentials::update_credential(
                        &account.creds.email,
                        &account.creds,
                        &self.cfg,
                    );
                }
                "t" => {
                    account.creds.tags = prompt("Tags: ");
                    let _ = credentials::update_credential(
                        &account.creds.email,
                        &account.creds,
                        &self.cfg,
                    );
                }
                "d" => {
                    if prompt_yes_no("Delete this saved account?", false) {
                        credentials::delete_credential(
                            &account.creds.email,
                            &self.cfg.account_format,
                        )?;
                        println!("Deleted.");
                        return Ok(());
                    }
                }
                "b" | "" => return Ok(()),
                _ => {}
            }
        }
    }

    async fn account_management(&mut self) -> Result<()> {
        let items = [
            "Keep alive / check all",
            "Storage info",
            "Delete a MEGA account",
            "Change password",
            "Create folder",
            "Back",
        ];
        let Some(choice) = select("Account Management", &items)? else {
            return Ok(());
        };
        match choice {
            0 => {
                crate::services::alive::check_all(
                    &self.cfg,
                    self.cfg.max_attempts as usize,
                    false,
                    true,
                    true,
                )
                .await?;
            }
            1 => self.storage_info().await?,
            2 => self.delete_account().await?,
            3 => services::account::change_password(),
            4 => self.create_folder().await?,
            _ => {}
        }
        Ok(())
    }

    fn pick_account(&self) -> Option<Credentials> {
        let accounts = credentials::load_all(&self.cfg.encryption_password);
        if accounts.is_empty() {
            println!("No saved accounts.");
            return None;
        }
        for (i, account) in accounts.iter().enumerate() {
            println!("{:>3}. {}", i + 1, account.creds.email);
        }
        let choice = prompt("Account number: ");
        let index = choice.parse::<usize>().ok()?;
        accounts.get(index.checked_sub(1)?).map(|a| a.creds.clone())
    }

    async fn storage_info(&mut self) -> Result<()> {
        let Some(creds) = self.pick_account() else {
            return Ok(());
        };
        let client = services::upload::get_session(&self.cfg, &creds).await?;
        let (used, total) = client.quota().await?;
        println!(
            "Storage: {} used of {}",
            crate::util::human_size(used),
            crate::util::human_size(total)
        );
        Ok(())
    }

    async fn delete_account(&mut self) -> Result<()> {
        let Some(creds) = self.pick_account() else {
            return Ok(());
        };
        if !prompt_yes_no("Delete this MEGA account permanently?", false) {
            return Ok(());
        }
        services::account::delete_account(&self.cfg, &creds).await?;
        let _ = credentials::delete_credential(&creds.email, &self.cfg.account_format);
        Ok(())
    }

    async fn create_folder(&mut self) -> Result<()> {
        let Some(creds) = self.pick_account() else {
            return Ok(());
        };
        let name = prompt("Folder name: ");
        if !name.trim().is_empty() {
            services::account::create_folder(&self.cfg, &creds, name.trim()).await?;
        }
        Ok(())
    }

    async fn cloud_menu(&mut self) -> Result<()> {
        let items = [
            "Upload a file",
            "Upload a directory",
            "Browse cloud files",
            "Back",
        ];
        let Some(choice) = select("Upload & Cloud", &items)? else {
            return Ok(());
        };
        if choice == 3 {
            return Ok(());
        }
        let Some(creds) = self.pick_account() else {
            return Ok(());
        };
        match choice {
            0 => {
                let file = prompt("File path: ");
                if !file.trim().is_empty() {
                    services::upload::upload_file(
                        &self.cfg,
                        &creds,
                        Path::new(&file),
                        false,
                    )
                    .await?;
                }
            }
            1 => {
                let dir = prompt("Directory: ");
                if !dir.trim().is_empty() {
                    services::upload::upload_dir(&self.cfg, &creds, Path::new(&dir), false).await?;
                }
            }
            2 => services::download::browse_cloud(&self.cfg, &creds).await?,
            _ => {}
        }
        Ok(())
    }

    async fn settings(&mut self) -> Result<()> {
        loop {
            let items = [
                "Max attempts",
                "Toggle visible browser",
                "Toggle auto CSV export",
                "Toggle auto JSONL export",
                "Generate a password",
                "Edit configuration",
                "Download Chromium",
                "Back",
            ];
            let Some(choice) = select("Settings", &items)? else {
                return Ok(());
            };
            match choice {
                0 => {
                    let value = prompt(&format!("Max attempts [{}]: ", self.cfg.max_attempts));
                    if let Ok(v) = value.parse::<u32>() {
                        self.cfg.max_attempts = v;
                        crate::config::save(self.profile.as_deref(), &self.cfg)?;
                    }
                }
                1 => {
                    self.cfg.visible_browser = !self.cfg.visible_browser;
                    crate::config::save(self.profile.as_deref(), &self.cfg)?;
                    println!("Visible browser: {}", self.cfg.visible_browser);
                }
                2 => {
                    self.cfg.csv_export = !self.cfg.csv_export;
                    crate::config::save(self.profile.as_deref(), &self.cfg)?;
                    println!("Auto CSV: {}", self.cfg.csv_export);
                }
                3 => {
                    self.cfg.jsonl_export = !self.cfg.jsonl_export;
                    crate::config::save(self.profile.as_deref(), &self.cfg)?;
                    println!("Auto JSONL: {}", self.cfg.jsonl_export);
                }
                4 => {
                    let pw = password::generate_password(20, 2, 3);
                    let (bits, label, _) = password::strength(&pw);
                    println!("{pw}  ({label}, {bits:.1} bits)");
                }
                5 => {
                    crate::config::edit(self.profile.as_deref());
                    self.cfg = crate::config::load(self.profile.as_deref());
                }
                6 => {
                    println!("Opening the Chromium download page...");
                    let _ = open_url("https://www.google.com/chrome/");
                }
                _ => return Ok(()),
            }
        }
    }
}

fn mask(value: &str) -> String {
    if value.chars().count() <= 2 {
        "*".repeat(value.chars().count())
    } else {
        let count = value.chars().count();
        format!("{}{}", "*".repeat(count - 2), value.chars().skip(count - 2).collect::<String>())
    }
}

fn open_url(url: &str) -> Result<()> {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd").args(["/C", "start", "", url]).spawn()?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open").arg(url).spawn()?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(url).spawn()?;
    }
    Ok(())
}

fn draw_menu(title: &str, items: &[&str], selected: usize) {
    execute!(
        std::io::stdout(),
        Clear(ClearType::All),
        MoveTo(0, 0)
    )
    .ok();
    let width = 52usize;
    let unicode = crate::console::unicode_enabled();
    let (tl, tr, bl, br, h, v, l, r) = if unicode {
        ("╔", "╗", "╚", "╝", "═", "║", "╠", "╣")
    } else {
        ("+", "+", "+", "+", "=", "|", "+", "+")
    };
    println!("{tl}{}{tr}", h.repeat(width - 2));
    let title_line = format!(" {title} ");
    let pad = width.saturating_sub(title_line.chars().count() + 2);
    println!("{v} {title_line}{}{v}", " ".repeat(pad));
    println!("{l}{}{r}", h.repeat(width - 2));
    for (i, item) in items.iter().enumerate() {
        let marker = if i == selected { ">" } else { " " };
        println!("{v} {marker} {:<46} {v}", item);
    }
    println!("{bl}{}{br}", h.repeat(width - 2));
    println!("  Up/Down or j/k, Enter to select, q to exit");
    let _ = std::io::stdout().flush();
}

fn select(title: &str, items: &[&str]) -> Result<Option<usize>> {
    use std::io::IsTerminal;
    if !std::io::stdout().is_terminal() {
        println!("{title}:");
        for (i, item) in items.iter().enumerate() {
            println!("  {}. {}", i + 1, item);
        }
        let answer = prompt("Choice: ");
        return Ok(answer
            .parse::<usize>()
            .ok()
            .and_then(|n| n.checked_sub(1))
            .filter(|n| *n < items.len()));
    }

    enable_raw_mode()?;
    execute!(std::io::stdout(), Hide)?;
    let mut selected = 0usize;
    let result = loop {
        draw_menu(title, items, selected);
        match read() {
            Ok(Event::Key(key)) if key.kind == KeyEventKind::Press => match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    selected = selected.saturating_sub(1);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    selected = (selected + 1).min(items.len() - 1);
                }
                KeyCode::Enter => break Ok(Some(selected)),
                KeyCode::Esc | KeyCode::Char('q') => break Ok(None),
                KeyCode::Char(c) if c.is_ascii_digit() => {
                    if let Some(index) = c.to_digit(10).map(|d| d as usize) {
                        if index >= 1 && index <= items.len() {
                            selected = index - 1;
                            break Ok(Some(selected));
                        }
                    }
                }
                KeyCode::Char('c')
                    if key
                        .modifiers
                        .contains(crossterm::event::KeyModifiers::CONTROL) =>
                {
                    break Ok(None);
                }
                _ => {}
            },
            Ok(_) => {}
            Err(e) => break Err(anyhow!("input error: {e}")),
        }
    };
    execute!(std::io::stdout(), Show)?;
    disable_raw_mode()?;
    println!();
    result
}

fn _now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

fn _unused(_: PathBuf) {}
