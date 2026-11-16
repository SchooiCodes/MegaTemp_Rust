use anyhow::{anyhow, bail, Context, Result};
use regex::Regex;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::browser::cdp::Browser;
use crate::browser::detect::detect_browser;
use crate::console::{p_print, send_webhook, separator, Color};
use crate::credentials;
use crate::mail::{self, MailProvider};
use crate::mega::MegaClient;
use crate::models::{Config, Credentials};
use crate::password::generate_password;
use crate::util::{random_first_name, random_alnum};

#[derive(Debug, Clone)]
pub struct RegisterOptions {
    pub cfg: Config,
    pub provider: String,
    pub proxy: Option<String>,
    pub visible: bool,
    pub browser: PathBuf,
    pub keep_profile: bool,
}

#[derive(Debug, Clone)]
pub struct RegisteredAccount {
    pub email: String,
    pub password: String,
    pub email_password: String,
}

/// Resolve a browser path from CLI/config/auto-detection.
pub fn resolve_browser(cfg: &Config, cli_path: Option<&str>) -> Option<PathBuf> {
    if let Some(path) = cli_path {
        let path = PathBuf::from(crate::util::expand_user(path));
        if path.is_file() {
            return Some(path);
        }
    }
    if !cfg.executable_path.is_empty() {
        let path = PathBuf::from(&cfg.executable_path);
        if path.is_file() {
            return Some(path);
        }
    }
    detect_browser()
}

pub async fn register_one(options: &RegisterOptions) -> Result<RegisteredAccount> {
    let started = std::time::Instant::now();
    let provider = MailProvider::from_name(&options.provider)?;
    // Warn once per process: repeating it on every retry of every worker drowned
    // out the lines that actually matter.
    static WARNED_GUERRILLA: std::sync::atomic::AtomicBool =
        std::sync::atomic::AtomicBool::new(false);
    if provider.name() == "guerrillamail"
        && !WARNED_GUERRILLA.swap(true, std::sync::atomic::Ordering::Relaxed)
    {
        p_print(
            "Note: Guerrilla Mail stores MEGA's verification email without a body; prefer --provider mailtm",
            Color::Warning,
        );
    }
    p_print(
        &format!("Using {} for the confirmation email", provider.name()),
        Color::Cyan,
    );
    let mailbox = provider.create().await.context("creating mailbox")?;
    p_print(&format!("Inbox: {}", mailbox.address), Color::Blue);
    crate::console::v_print(
        &format!(
            "[mail] {} inbox {} secret set={}",
            provider.name(),
            mailbox.address,
            !mailbox.secret.is_empty()
        ),
        Color::Cyan,
    );

    let password = generate_password(20, 2, 3);
    let profile_name = format!(
        "tmp_{}",
        random_alnum(8)
    );
    let profile_dir = crate::disk::profile_dir(&profile_name);
    // Fall back to the rotating proxy manager when no proxy was passed in.
    let launch_proxy = options.proxy.clone().or_else(crate::proxy::current);
    let browser = Browser::launch(
        &options.browser,
        !options.visible,
        &profile_dir,
        launch_proxy.as_deref(),
    )
    .await
    .context("starting browser")?;

    let result = run_registration(options, &provider, &mailbox, &password, &browser).await;

    match result {
        Ok(account) => {
            let creds = Credentials {
                email: account.email.clone(),
                email_password: account.email_password.clone(),
                password: account.password.clone(),
                last_login: now(),
                ..Default::default()
            };
            match credentials::save_credentials(&creds, &options.cfg) {
                Ok(path) => {
                    separator("Success", 60);
                    p_print(&format!("  Account:      {}", account.email), Color::Ok);
                    p_print(&format!("  Password:     {}", account.password), Color::Ok);
                    p_print(
                        &format!("  Verified in:  {}", crate::console::elapsed(started)),
                        Color::Cyan,
                    );
                    p_print(&format!("  Saved to:     {}", path.display()), Color::Ok);
                }
                Err(e) => {
                    separator("Created, but not saved", 60);
                    p_print(&format!("  Account:      {}", account.email), Color::Ok);
                    p_print(&format!("  Password:     {}", account.password), Color::Warning);
                    p_print(
                        &format!("  Verified in:  {}", crate::console::elapsed(started)),
                        Color::Cyan,
                    );
                    p_print(&format!("  Could not save credentials: {e}"), Color::Fail);
                }
            }
            if !options.cfg.webhook_url.is_empty() {
                send_webhook(
                    &options.cfg.webhook_url,
                    "registration_success",
                    serde_json::json!({"email": account.email}),
                );
            }
            browser.close().await;
            if !options.keep_profile {
                let _ = std::fs::remove_dir_all(&profile_dir);
            }
            Ok(account)
        }
        Err(e) => {
            if !options.cfg.webhook_url.is_empty() {
                send_webhook(
                    &options.cfg.webhook_url,
                    "registration_failed",
                    serde_json::json!({"error": e.to_string()}),
                );
            }
            browser.close().await;
            let _ = std::fs::remove_dir_all(&profile_dir);
            Err(e)
        }
    }
}

async fn run_registration(
    options: &RegisterOptions,
    provider: &MailProvider,
    mailbox: &mail::Mailbox,
    password: &str,
    browser: &Browser,
) -> Result<RegisteredAccount> {
    let page = browser.new_page("https://mega.nz/register").await?;

    let ready = page
        .wait_selector("#register-firstname", Duration::from_secs(15))
        .await?;
    if !ready {
        let body = page.body_text().await.unwrap_or_default();
        let hint = if body.to_lowercase().contains("too many") {
            "MEGA is rate limiting registrations"
        } else if body.trim().len() < 40 {
            "the registration page did not load"
        } else {
            "the registration form did not appear"
        };
        bail!("{hint}");
    }

    page.type_into("#register-firstname", random_first_name()).await?;
    page.type_into("#register-email", &mailbox.address).await?;
    page.type_into("#register-password", password).await?;
    p_print("Registration form submitted", Color::Cyan);

    page.click_if_exists(".privacy-check").await?;
    if !page
        .wait_selector(".register-button", Duration::from_secs(10))
        .await?
    {
        bail!("the register button did not appear");
    }
    page.click(".register-button").await?;

    p_print("Waiting for MEGA's confirmation email", Color::Cyan);
    let message = mail::wait_for_message(provider, mailbox, options.cfg.mail_timeout)
        .await
        .context("waiting for confirmation email")?;
    p_print(
        &format!("Verification email received ({})", message.mail_subject),
        Color::Green,
    );

    let confirm = extract_confirm_link(&message.mail_body)
        .ok_or_else(|| anyhow!("confirmation email contained no MEGA link"))?;
    let confirm_page = browser.new_page(&confirm).await?;

    // The link often redirects straight to the password step.
    let _ = confirm_page
        .wait_url_not_contains("#confirm", Duration::from_secs(3))
        .await;
    confirm_page.wait_ready().await?;

    let selectors = ["#login-password2", "#confirm-password2", "#password"];
    let mut chosen = None;
    for selector in selectors {
        if confirm_page
            .wait_selector(selector, Duration::from_secs(3))
            .await?
        {
            chosen = Some(selector);
            break;
        }
    }
    let Some(selector) = chosen else {
        let body = confirm_page.body_text().await.unwrap_or_default();
        let lower = body.to_lowercase();
        let hint = if lower.contains("captcha") {
            "a captcha was shown"
        } else if lower.contains("phone") {
            "MEGA requested phone verification"
        } else if lower.contains("error") {
            "MEGA returned an error during confirmation"
        } else {
            "the confirmation password field did not appear"
        };
        bail!("{hint}");
    };

    confirm_page.type_into(selector, password).await?;
    let clicked = confirm_page.click_text("button", "Confirm").await?
        || confirm_page.click_text("a", "Confirm").await?
        || confirm_page.click(".login-button").await?
        || confirm_page.click(".register-button").await?
        || confirm_page.click("#login-button2").await?;
    if !clicked {
        bail!("could not find the confirm button");
    }

    if !confirm_page
        .wait_url_not_contains("confirm", Duration::from_secs(12))
        .await?
    {
        let errors: Vec<String> = confirm_page
            .eval(
                r#"Array.from(document.querySelectorAll('.error,.warning,.msg,.input-error,.toast,.notification')).map(e => e.innerText.trim()).filter(Boolean)"#,
            )
            .await
            .ok()
            .and_then(|v| v.as_array().cloned())
            .map(|items| {
                items
                    .into_iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        bail!(
            "confirmation did not complete{}",
            if errors.is_empty() {
                String::new()
            } else {
                format!(": {}", errors.join("; "))
            }
        );
    }

    if confirm_page.exists(".dialog-download-recovery-key").await? {
        let _ = confirm_page.click_text("button", "Later").await;
        let _ = confirm_page.click_text("button", "Skip").await;
        tokio::time::sleep(Duration::from_millis(300)).await;
    }

    // Best-effort: remove the default welcome file.
    if let Ok(client) = login_client(&options.cfg, &mailbox.address, password).await {
        let _ = client.delete_welcome_file().await;
    }

    confirm_page.close().await;
    page.close().await;

    Ok(RegisteredAccount {
        email: mailbox.address.clone(),
        password: password.to_string(),
        email_password: mailbox.secret.clone(),
    })
}

async fn login_client(cfg: &Config, email: &str, password: &str) -> Result<MegaClient> {
    let proxy = if cfg.proxy.is_empty() {
        None
    } else {
        Some(cfg.proxy.as_str())
    };
    let mut client = MegaClient::new(proxy)?;
    client.login(email, password).await?;
    Ok(client)
}

fn extract_confirm_link(body: &str) -> Option<String> {
    let patterns = [
        r#"href="(https://mega\.nz/#confirm[^"]*)""#,
        r#"(https://mega\.nz/#confirm[^\s"'<>]+)"#,
        r#"(https://mega\.nz/confirm[^\s"'<>]+)"#,
    ];
    for pattern in patterns {
        if let Ok(re) = Regex::new(pattern) {
            if let Some(caps) = re.captures(body) {
                let link = caps.get(1)?.as_str().replace("&amp;", "&");
                return Some(link);
            }
        }
    }
    None
}

fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// Register with retries and provider fallback.
pub async fn register_with_retries(options: &RegisterOptions) -> Result<RegisteredAccount> {
    let primary = options.provider.clone();
    let fallback = mail::fallback_provider(&primary).to_string();
    let mut last_error = None;

    for provider_name in [primary.clone(), fallback.clone()] {
        for attempt in 1..=options.cfg.max_attempts.max(1) {
            p_print(
                &format!(
                    "Attempt {attempt}/{} ({provider_name})",
                    options.cfg.max_attempts
                ),
                Color::Header,
            );
            let mut attempt_options = options.clone();
            attempt_options.provider = provider_name.clone();
            match register_one(&attempt_options).await {
                Ok(account) => return Ok(account),
                Err(e) => {
                    let rate_limited = mail::is_rate_limited(&e);
                    let unusable = mail::is_unusable(&e);
                    p_print(&format!("Attempt failed: {e:#}"), Color::Warning);
                    last_error = Some(e);
                    if rate_limited || unusable {
                        let reason = if rate_limited {
                            "rate limited"
                        } else {
                            "cannot render MEGA's verification email"
                        };
                        p_print(
                            &format!(
                                "Skipping {provider_name} ({reason}); use --provider {} to pick a different one",
                                mail::fallback_provider(&provider_name)
                            ),
                            Color::Warning,
                        );
                        break;
                    }
                    let delay = (1.0 * 1.5f64.powi(attempt as i32 - 1)).min(3.0);
                    crate::console::v_print(
                        &format!("[retry] {provider_name} again in {delay:.1}s"),
                        Color::Cyan,
                    );
                    tokio::time::sleep(Duration::from_secs_f64(delay)).await;
                }
            }
        }
    }

    Err(last_error.unwrap_or_else(|| anyhow!("all registration attempts failed")))
}
