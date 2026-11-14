use anyhow::Result;
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

use crate::console::{col, p_print, Color};
use crate::models::{Config, CONFIG_KEYS, CURRENT_SCHEMA};

pub fn config_path(profile: Option<&str>) -> PathBuf {
    match profile {
        Some(p) if !p.is_empty() => PathBuf::from(format!("config-{p}.json")),
        _ => PathBuf::from("config.json"),
    }
}

/// Serialize with the 4-space indentation used by the Python implementation.
pub fn to_pretty(value: &Value, indent: usize) -> String {
    let pad = " ".repeat(indent);
    let formatter = serde_json::ser::PrettyFormatter::with_indent(pad.as_bytes());
    let mut out = Vec::new();
    let mut ser = serde_json::Serializer::with_formatter(&mut out, formatter);
    serde::Serialize::serialize(value, &mut ser).expect("serialize json");
    String::from_utf8(out).expect("json is utf-8")
}

fn apply_defaults(map: &mut Map<String, Value>) -> bool {
    let defaults = serde_json::to_value(Config::default()).unwrap();
    let default_map = defaults.as_object().unwrap();
    let mut changed = false;
    for (k, v) in default_map {
        if !map.contains_key(k) {
            map.insert(k.clone(), v.clone());
            changed = true;
        }
    }
    changed
}

/// Load a config file. Missing files return defaults without creating anything.
pub fn load(profile: Option<&str>) -> Config {
    let path = config_path(profile);
    read_from(&path, profile)
}

pub fn read_from(path: &Path, profile: Option<&str>) -> Config {
    let raw = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Config::default(),
        Err(e) => {
            p_print(&format!("Warning: could not read {}: {e}", path.display()), Color::Warning);
            return Config::default();
        }
    };

    if raw.trim().is_empty() {
        let cfg = Config::default();
        let _ = save_to(path, &cfg);
        return cfg;
    }

    let mut value: Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            p_print(
                &format!("Warning: invalid config {}: {e}; using defaults", path.display()),
                Color::Warning,
            );
            let bak = path.with_extension(format!(
                "{}.bak",
                path.extension().and_then(|s| s.to_str()).unwrap_or("json")
            ));
            let _ = std::fs::rename(path, &bak);
            let cfg = Config::default();
            let _ = save_to(path, &cfg);
            return cfg;
        }
    };

    let mut map = match value.as_object_mut() {
        Some(m) => std::mem::take(m),
        None => {
            p_print("Warning: config root is not an object; using defaults", Color::Warning);
            return Config::default();
        }
    };

    // Unknown keys are warned about and dropped, matching the Python reader.
    let unknown: Vec<String> = map
        .keys()
        .filter(|k| !CONFIG_KEYS.contains(&k.as_str()))
        .cloned()
        .collect();
    for k in &unknown {
        p_print(&format!("Warning: unknown config key '{k}' ignored"), Color::Warning);
        map.remove(k);
    }

    let mut changed = apply_defaults(&mut map);

    let schema = map
        .get("schemaVersion")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;
    if schema < CURRENT_SCHEMA {
        map.insert("schemaVersion".into(), Value::from(CURRENT_SCHEMA));
        changed = true;
    }

    value = Value::Object(map);
    let cfg: Config = match serde_json::from_value(value) {
        Ok(c) => c,
        Err(e) => {
            p_print(&format!("Warning: invalid config values: {e}; using defaults"), Color::Warning);
            Config::default()
        }
    };

    if changed {
        let _ = save_to(path, &cfg);
    }

    validate(&cfg, profile);
    cfg
}

pub fn save_to(path: &Path, cfg: &Config) -> Result<()> {
    let value = serde_json::to_value(cfg)?;
    let text = to_pretty(&value, 4);
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text + "\n")?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

pub fn save(profile: Option<&str>, cfg: &Config) -> Result<()> {
    save_to(&config_path(profile), cfg)
}

pub fn validate(cfg: &Config, profile: Option<&str>) {
    if !cfg.executable_path.is_empty() && !Path::new(&cfg.executable_path).exists() {
        p_print(
            &format!(
                "{}Warning:{} executablePath does not exist: {}",
                col(Color::Warning),
                col(Color::End),
                cfg.executable_path
            ),
            Color::Warning,
        );
    }
    if !(1..=50).contains(&cfg.max_attempts) {
        p_print(
            &format!(
                "Warning: maxAttempts {} is outside the recommended range 1..50",
                cfg.max_attempts
            ),
            Color::Warning,
        );
    }
    if !cfg.proxy.is_empty() {
        let scheme = cfg.proxy.split("://").next().unwrap_or("");
        let valid = ["http", "https", "socks4", "socks5"];
        if !cfg.proxy.contains("://") || !valid.contains(&scheme) {
            p_print(
                &format!("Warning: proxy '{}' lacks a supported scheme", cfg.proxy),
                Color::Warning,
            );
        }
    }
    let _ = profile;
}

/// Interactive "edit config" helper (settings menu).
pub fn edit(profile: Option<&str>) {
    let mut cfg = load(profile);
    println!("Editing configuration (leave blank to keep current value)");
    cfg.account_format = prompt_default("Account format", &cfg.account_format);
    cfg.executable_path = prompt_default("Browser executable", &cfg.executable_path);
    cfg.email_provider = prompt_default("Email provider", &cfg.email_provider);
    if let Some(v) = prompt_default("Max attempts", &cfg.max_attempts.to_string()).parse().ok() {
        cfg.max_attempts = v;
    }
    if let Some(v) = prompt_default("Mail timeout (seconds)", &cfg.mail_timeout.to_string())
        .parse()
        .ok()
    {
        cfg.mail_timeout = v;
    }
    cfg.webhook_url = prompt_default("Webhook URL", &cfg.webhook_url);
    if let Err(e) = save(profile, &cfg) {
        p_print(&format!("Could not save config: {e}"), Color::Fail);
    } else {
        p_print("Configuration saved.", Color::Ok);
    }
}

fn prompt_default(label: &str, current: &str) -> String {
    use std::io::Write;
    print!("{label} [{current}]: ");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    let _ = std::io::stdin().read_line(&mut line);
    let line = line.trim().to_string();
    if line.is_empty() {
        current.to_string()
    } else {
        line
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let cfg = Config::default();
        save_to(&path, &cfg).unwrap();
        let loaded = read_from(&path, None);
        assert_eq!(cfg, loaded);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"schemaVersion\": 3"));
    }

    #[test]
    fn migrates_missing_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, "{\"schemaVersion\":0,\"maxAttempts\":7}").unwrap();
        let cfg = read_from(&path, None);
        assert_eq!(cfg.schema_version, 3);
        assert_eq!(cfg.max_attempts, 7);
        assert_eq!(cfg.email_provider, "mailtm");
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("encryptionPassword"));
    }

    #[test]
    fn corrupt_config_is_backed_up() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, "{not json").unwrap();
        let cfg = read_from(&path, None);
        assert_eq!(cfg.schema_version, 3);
        assert!(dir.path().join("config.json.bak").exists() || dir.path().join("config.bak").exists());
    }
}
