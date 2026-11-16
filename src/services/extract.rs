use anyhow::Result;
use std::path::PathBuf;

use crate::console::{p_print, Color};
use crate::credentials;
use crate::models::Config;

pub fn extract_credentials(cfg: &Config, format: &str) -> Result<PathBuf> {
    p_print("Extracting credentials to credentials.txt ...", Color::Header);
    let accounts = credentials::load_all(&cfg.encryption_password, &cfg.account_format);
    let format = if format.is_empty() {
        "{email}#{password}"
    } else {
        format
    };
    let path = std::path::PathBuf::from(credentials::CREDENTIALS_TXT);
    let mut lines = Vec::new();
    for account in &accounts {
        if account.decrypt_failed {
            p_print(
                &format!("Skipping {} (could not decrypt)", account.creds.email),
                Color::Warning,
            );
            continue;
        }
        let line = format
            .replace("{email}", &account.creds.email)
            .replace("{password}", &account.creds.password)
            .replace("{emailPassword}", &account.creds.email_password);
        lines.push(line);
    }
    std::fs::write(&path, lines.join("\n") + if lines.is_empty() { "" } else { "\n" })?;
    let absolute = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
    let shown = absolute.display().to_string();
    let shown = shown.strip_prefix(r"\\?\").unwrap_or(&shown);
    p_print(
        &format!(
            "Extracted {} account(s) to {}",
            lines.len(),
            shown
        ),
        Color::Ok,
    );
    Ok(path)
}

fn writable_accounts(cfg: &Config) -> Vec<credentials::LoadedCredential> {
    credentials::load_all(&cfg.encryption_password, &cfg.account_format)
        .into_iter()
        .filter(|a| !a.decrypt_failed)
        .collect()
}

pub fn export_bitwarden(cfg: &Config) -> Result<PathBuf> {
    let path = PathBuf::from("bitwarden_export.csv");
    let mut writer = csv::Writer::from_path(&path)?;
    writer.write_record([
        "folder",
        "favorite",
        "type",
        "name",
        "notes",
        "login_uri",
        "login_username",
        "login_password",
        "login_totp",
    ])?;
    for account in writable_accounts(cfg) {
        writer.write_record([
            "",
            "0",
            "1",
            &account.creds.email,
            &account.creds.notes,
            "https://mega.nz",
            &account.creds.email,
            &account.creds.password,
            "",
        ])?;
    }
    writer.flush()?;
    p_print(&format!("Wrote {}", path.display()), Color::Ok);
    Ok(path)
}

pub fn export_onepassword(cfg: &Config) -> Result<PathBuf> {
    let path = PathBuf::from("1password_export.csv");
    let mut writer = csv::Writer::from_path(&path)?;
    writer.write_record(["title", "url", "username", "password", "notes"])?;
    for account in writable_accounts(cfg) {
        writer.write_record([
            &format!("MEGA - {}", account.creds.email),
            "https://mega.nz",
            &account.creds.email,
            &account.creds.password,
            &account.creds.notes,
        ])?;
    }
    writer.flush()?;
    p_print(&format!("Wrote {}", path.display()), Color::Ok);
    Ok(path)
}

pub fn export_keepass(cfg: &Config) -> Result<PathBuf> {
    let path = PathBuf::from("keepass_export.csv");
    let mut writer = csv::Writer::from_path(&path)?;
    writer.write_record(["Group", "Title", "Username", "Password", "URL", "Notes"])?;
    for account in writable_accounts(cfg) {
        writer.write_record([
            "MEGA",
            &account.creds.email,
            &account.creds.email,
            &account.creds.password,
            "https://mega.nz",
            &account.creds.notes,
        ])?;
    }
    writer.flush()?;
    p_print(&format!("Wrote {}", path.display()), Color::Ok);
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_are_sane() {
        let cfg = Config::default();
        // Never hit the network; just ensure the filename constants are stable.
        assert_eq!(cfg.account_format, "");
        assert_eq!(credentials::CREDENTIALS_TXT, "credentials.txt");
    }
}
