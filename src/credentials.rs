use anyhow::{anyhow, bail, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::config::to_pretty;
use crate::console::{p_print, Color};
use crate::crypto::{decrypt_field, encrypt_field, is_encrypted_value};
use crate::lock;
use crate::models::{Config, Credentials};

pub const CREDENTIALS_DIR: &str = "credentials";
pub const CREDENTIALS_TXT: &str = "credentials.txt";

pub fn dir() -> PathBuf {
    PathBuf::from(CREDENTIALS_DIR)
}

pub fn sanitize_filename(email: &str) -> String {
    let local = email.split('@').next().unwrap_or(email);
    if local.is_empty() {
        return "account".to_string();
    }
    let mut out = String::new();
    for c in local.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ' ') {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        "account".to_string()
    } else {
        out
    }
}

pub fn credential_path(email: &str) -> PathBuf {
    dir().join(format!("{}.json", sanitize_filename(email)))
}

fn mtime(path: &Path) -> f64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

#[derive(Debug, Clone)]
pub struct LoadedCredential {
    pub path: PathBuf,
    pub creds: Credentials,
    pub encrypted: bool,
    pub decrypt_failed: bool,
    pub mtime: f64,
}

/// Load one credential file, decrypting fields when necessary.
pub fn load_file(path: &Path, password: &str) -> Result<LoadedCredential> {
    let text = std::fs::read_to_string(path)?;
    let mut creds: Credentials = serde_json::from_str(&text)?;
    let encrypted = is_encrypted_value(&creds.password) || is_encrypted_value(&creds.email_password);
    let mut decrypt_failed = false;
    if encrypted {
        if password.is_empty() {
            p_print(
                &format!(
                    "Warning: {} is encrypted but no encryption password is configured",
                    path.display()
                ),
                Color::Warning,
            );
            creds.password.clear();
            creds.email_password.clear();
            decrypt_failed = true;
        } else {
            if !creds.password.is_empty() {
                match decrypt_field(&creds.password, password) {
                    Ok(v) => creds.password = v,
                    Err(e) => {
                        p_print(
                            &format!("Error: could not decrypt {}: {e}", path.display()),
                            Color::Fail,
                        );
                        creds.password.clear();
                        decrypt_failed = true;
                    }
                }
            }
            if !creds.email_password.is_empty() {
                match decrypt_field(&creds.email_password, password) {
                    Ok(v) => creds.email_password = v,
                    Err(e) => {
                        p_print(
                            &format!("Error: could not decrypt {}: {e}", path.display()),
                            Color::Fail,
                        );
                        creds.email_password.clear();
                        decrypt_failed = true;
                    }
                }
            }
        }
    }
    Ok(LoadedCredential {
        path: path.to_path_buf(),
        creds,
        encrypted,
        decrypt_failed,
        mtime: mtime(path),
    })
}

/// Load every JSON account, newest first.
pub fn load_all(password: &str) -> Vec<LoadedCredential> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir()) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        match load_file(&path, password) {
            Ok(item) => out.push(item),
            Err(e) => p_print(
                &format!("Warning: skipping {}: {e}", path.display()),
                Color::Warning,
            ),
        }
    }
    out.sort_by(|a, b| b.mtime.partial_cmp(&a.mtime).unwrap_or(std::cmp::Ordering::Equal));
    out
}

/// Credentials stored only in accounts.txt (not yet represented by JSON).
pub fn accounts_txt_entries(format: &str, password: &str) -> Vec<LoadedCredential> {
    let path = dir().join("accounts.txt");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let format = if format.is_empty() {
        "{email}#{password}"
    } else {
        format
    };
    let re = account_format_regex(format);
    let mut out = Vec::new();
    for line in text.lines() {
        let Some(caps) = re.as_ref().and_then(|r| r.captures(line)) else {
            continue;
        };
        let get = |name: &str| {
            caps.name(name)
                .map(|m| m.as_str().to_string())
                .unwrap_or_default()
        };
        let email = get("email");
        if email.is_empty() {
            continue;
        }
        let mut creds = Credentials::new(email, get("password"));
        creds.email_password = get("emailPassword");
        out.push(LoadedCredential {
            path: path.clone(),
            creds,
            encrypted: false,
            decrypt_failed: false,
            mtime: mtime(&path),
        });
    }
    let _ = password;
    out
}

fn account_format_regex(format: &str) -> Option<regex::Regex> {
    let mut pattern = String::from("^");
    let mut rest = format;
    while let Some(pos) = rest.find('{') {
        pattern.push_str(&regex::escape(&rest[..pos]));
        let after = &rest[pos + 1..];
        let Some(end) = after.find('}') else {
            pattern.push_str(&regex::escape(&rest[pos..]));
            rest = "";
            break;
        };
        let name = &after[..end];
        pattern.push_str(&format!("(?P<{}>.*?)", regex::escape(name)));
        rest = &after[end + 1..];
    }
    pattern.push_str(&regex::escape(rest));
    pattern.push('$');
    regex::Regex::new(&pattern).ok()
}

/// Save a new account: JSON plus optional accounts.txt / CSV / JSONL exports.
pub fn save_credentials(creds: &Credentials, cfg: &Config) -> Result<PathBuf> {
    std::fs::create_dir_all(dir())?;
    let path = credential_path(&creds.email);
    let mut stored = creds.clone();
    if !cfg.encryption_password.is_empty() {
        stored.password = encrypt_field(&stored.password, &cfg.encryption_password)?;
        stored.email_password = encrypt_field(&stored.email_password, &cfg.encryption_password)?;
    }
    let value = serde_json::to_value(&stored)?;
    let text = to_pretty(&value, 2);
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text + "\n")?;
    std::fs::rename(&tmp, &path)?;

    if !cfg.account_format.is_empty() {
        append_account_format(creds, &cfg.account_format)?;
    }
    if cfg.csv_export {
        append_csv(creds)?;
    }
    if cfg.jsonl_export {
        append_jsonl(creds)?;
    }
    Ok(path)
}

fn render_account_line(creds: &Credentials, format: &str) -> String {
    format
        .replace("{email}", &creds.email)
        .replace("{password}", &creds.password)
        .replace("{emailPassword}", &creds.email_password)
}

fn append_account_format(creds: &Credentials, format: &str) -> Result<()> {
    std::fs::create_dir_all(dir())?;
    let path = dir().join("accounts.txt");
    let _guard = lock::acquire(&path)?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    use std::io::Write;
    writeln!(file, "{}", render_account_line(creds, format))?;
    Ok(())
}

pub fn append_csv(creds: &Credentials) -> Result<()> {
    std::fs::create_dir_all(dir())?;
    let path = dir().join("accounts.csv");
    let _guard = lock::acquire(&path)?;
    let new_file = !path.exists();
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    let mut writer = csv::Writer::from_writer(file);
    if new_file {
        writer.write_record(["email", "password", "emailPassword"])?;
    }
    writer.write_record([&creds.email, &creds.password, &creds.email_password])?;
    writer.flush()?;
    Ok(())
}

pub fn append_jsonl(creds: &Credentials) -> Result<()> {
    std::fs::create_dir_all(dir())?;
    let path = dir().join("accounts.jsonl");
    let _guard = lock::acquire(&path)?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    use std::io::Write;
    let line = serde_json::to_string(&serde_json::json!({
        "email": creds.email,
        "password": creds.password,
        "emailPassword": creds.email_password,
    }))?;
    writeln!(file, "{line}")?;
    Ok(())
}

/// Find an account file by exact path, email, filename stem, or accounts.txt.
pub fn find_credential_path(email: &str) -> Option<PathBuf> {
    let as_path = PathBuf::from(email);
    if as_path.is_file() {
        return Some(as_path);
    }
    let needle = email.to_lowercase();
    if let Ok(entries) = std::fs::read_dir(dir()) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            if let Ok(text) = std::fs::read_to_string(&path) {
                if let Ok(value) = serde_json::from_str::<Value>(&text) {
                    if value
                        .get("email")
                        .and_then(|v| v.as_str())
                        .map(|e| e.to_lowercase() == needle)
                        .unwrap_or(false)
                    {
                        return Some(path);
                    }
                }
            }
            if path
                .file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.to_lowercase() == needle)
                .unwrap_or(false)
            {
                return Some(path);
            }
        }
    }
    let candidate = credential_path(email);
    candidate.is_file().then_some(candidate)
}

fn rewrite_accounts_txt(email: &str, replacement: Option<&Credentials>, format: &str) -> Result<()> {
    let path = dir().join("accounts.txt");
    if !path.exists() {
        return Ok(());
    }
    let _guard = lock::acquire(&path)?;
    let text = std::fs::read_to_string(&path)?;
    let format = if format.is_empty() {
        "{email}#{password}"
    } else {
        format
    };
    let re = account_format_regex(format);
    let mut out = String::new();
    let needle = email.to_lowercase();
    for line in text.lines() {
        let matches = re
            .as_ref()
            .and_then(|r| r.captures(line))
            .map(|c| {
                c.name("email")
                    .map(|m| m.as_str().to_lowercase() == needle)
                    .unwrap_or(false)
            })
            .unwrap_or(false);
        if matches {
            if let Some(creds) = replacement {
                out.push_str(&render_account_line(creds, format));
                out.push('\n');
            }
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    std::fs::write(&path, out)?;
    Ok(())
}

/// Update fields on an existing account, preserving encryption.
pub fn update_credential(email: &str, fields: &Credentials, cfg: &Config) -> Result<PathBuf> {
    let path = find_credential_path(email).unwrap_or_else(|| credential_path(email));
    let mut current = if path.is_file() {
        load_file(&path, &cfg.encryption_password)?.creds
    } else {
        Credentials {
            email: email.to_string(),
            ..Default::default()
        }
    };

    if fields.email.is_empty() {
        // keep existing address
    } else {
        current.email = fields.email.clone();
    }
    if !fields.password.is_empty() {
        current.password = fields.password.clone();
    }
    if !fields.email_password.is_empty() {
        current.email_password = fields.email_password.clone();
    }
    current.id = fields.id.clone();
    current.notes = fields.notes.clone();
    current.tags = fields.tags.clone();
    if fields.last_login > 0.0 {
        current.last_login = fields.last_login;
    }

    // Rewrite in place (so renaming is not silently ignored) and re-encrypt.
    let mut stored = current.clone();
    if !cfg.encryption_password.is_empty() {
        stored.password = encrypt_field(&stored.password, &cfg.encryption_password)?;
        stored.email_password = encrypt_field(&stored.email_password, &cfg.encryption_password)?;
    }
    let value = serde_json::to_value(&stored)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, to_pretty(&value, 2) + "\n")?;
    std::fs::rename(&tmp, &path)?;
    rewrite_accounts_txt(email, Some(&current), &cfg.account_format)?;
    Ok(path)
}

/// Update only the login timestamp, preserving all other fields.
pub fn touch_last_login(email: &str, cfg: &Config, timestamp: f64) -> Result<()> {
    let path = find_credential_path(email).unwrap_or_else(|| credential_path(email));
    if !path.is_file() {
        return Ok(());
    }
    let mut current = load_file(&path, &cfg.encryption_password)?.creds;
    current.last_login = timestamp;
    let mut stored = current;
    if !cfg.encryption_password.is_empty() {
        stored.password = encrypt_field(&stored.password, &cfg.encryption_password)?;
        stored.email_password = encrypt_field(&stored.email_password, &cfg.encryption_password)?;
    }
    let value = serde_json::to_value(&stored)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, to_pretty(&value, 2) + "\n")?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

pub fn delete_credential(email: &str, format: &str) -> Result<bool> {
    let path = find_credential_path(email);
    let mut removed = false;
    if let Some(path) = path {
        if path.extension().and_then(|e| e.to_str()) == Some("json") && path.is_file() {
            std::fs::remove_file(&path)?;
            removed = true;
        }
    }
    rewrite_accounts_txt(email, None, format)?;
    Ok(removed)
}

pub fn set_encryption_password(profile: Option<&str>, password: &str) -> Result<()> {
    let mut cfg = crate::config::load(profile);
    cfg.encryption_password = password.to_string();
    crate::config::save(profile, &cfg)
}

/// Read an arbitrary credentials JSON file for exports / health checks.
pub fn read_export(path: &Path, password: &str) -> Result<Credentials> {
    Ok(load_file(path, password)?.creds)
}

pub fn require_email(creds: &Credentials) -> Result<()> {
    if creds.email.is_empty() {
        bail!("account has no email");
    }
    Ok(())
}

pub fn parsed_accounts_txt(format: &str) -> Result<Vec<Credentials>> {
    let path = dir().join("accounts.txt");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text = std::fs::read_to_string(&path)?;
    let format = if format.is_empty() {
        "{email}#{password}"
    } else {
        format
    };
    let re = account_format_regex(format).ok_or_else(|| anyhow!("invalid account format"))?;
    let mut out = Vec::new();
    for line in text.lines() {
        if let Some(caps) = re.captures(line) {
            let get = |name: &str| {
                caps.name(name)
                    .map(|m| m.as_str().to_string())
                    .unwrap_or_default()
            };
            out.push(Credentials {
                email: get("email"),
                password: get("password"),
                email_password: get("emailPassword"),
                ..Default::default()
            });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitises_filenames() {
        assert_eq!(sanitize_filename("john.doe+tag@example.com"), "john.doe_tag");
        assert_eq!(sanitize_filename("@@@"), "account");
    }

    #[test]
    fn account_format_roundtrip() {
        let creds = Credentials {
            email: "a@b.c".into(),
            password: "p#w".into(),
            ..Default::default()
        };
        assert_eq!(
            render_account_line(&creds, "{email}#{password}"),
            "a@b.c#p#w"
        );
    }

    #[test]
    fn parses_custom_format() {
        let re = account_format_regex("{email} | {password}").unwrap();
        let caps = re.captures("x@y.z | secret").unwrap();
        assert_eq!(&caps["email"], "x@y.z");
        assert_eq!(&caps["password"], "secret");
    }
}
