//! Download a Chromium build when no browser is installed.
//!
//! Python's tool shells out to `pyppeteer install`; the Rust port fetches
//! Chrome for Testing instead, so it does not depend on a Python runtime.

use anyhow::{bail, Context, Result};
use futures::StreamExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const LAST_KNOWN_GOOD: &str =
    "https://googlechromelabs.github.io/chrome-for-testing/last-known-good-versions-with-downloads.json";

/// Platform key used by the Chrome for Testing download index.
pub fn platform_key() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        "win64"
    }
    #[cfg(target_os = "macos")]
    {
        if cfg!(target_arch = "aarch64") {
            "mac-arm64"
        } else {
            "mac-x64"
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        "linux64"
    }
}

/// Where downloaded browsers live.
pub fn install_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("megatem")
        .join("chromium")
}

/// Download the stable Chrome for Testing build and return its executable.
pub async fn download_chromium() -> Result<PathBuf> {
    let dir = install_dir();
    if let Some(found) = find_chrome(&dir) {
        return Ok(found);
    }
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("creating {}", dir.display()))?;

    let client = reqwest::Client::builder()
        .user_agent("MegaTemp")
        .timeout(Duration::from_secs(600))
        .build()?;
    let url = stable_download_url(&client).await?;
    let zip = dir.join("chrome.zip");
    crate::console::status_line("Downloading Chromium...");
    let response = client.get(&url).send().await?.error_for_status()?;
    let total = response.content_length().unwrap_or(0);
    let mut file = tokio::fs::File::create(&zip).await?;
    {
        use tokio::io::AsyncWriteExt;
        let mut stream = response.bytes_stream();
        let mut downloaded = 0u64;
        let mut last = Instant::now();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            file.write_all(&chunk).await?;
            downloaded += chunk.len() as u64;
            if last.elapsed() >= Duration::from_millis(300) {
                let progress = if total > 0 {
                    format!("{:.0}/{:.0} MB", downloaded as f64 / 1e6, total as f64 / 1e6)
                } else {
                    format!("{:.0} MB", downloaded as f64 / 1e6)
                };
                crate::console::status_line(&format!("Downloading Chromium... {progress}"));
                last = Instant::now();
            }
        }
        file.flush().await?;
    }
    crate::console::status_line("Extracting Chromium...");
    extract_zip(&zip, &dir)?;
    let _ = std::fs::remove_file(&zip);
    crate::console::clear_status_line();

    let found = find_chrome(&dir).ok_or_else(|| {
        anyhow::anyhow!(
            "downloaded Chromium but could not find its executable under {}",
            dir.display()
        )
    })?;
    Ok(found)
}

/// Resolve the stable channel URL for this platform.
async fn stable_download_url(client: &reqwest::Client) -> Result<String> {
    let index: serde_json::Value = client
        .get(LAST_KNOWN_GOOD)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let key = platform_key();
    let downloads = index
        .get("channels")
        .and_then(|c| c.get("Stable"))
        .and_then(|c| c.get("downloads"))
        .and_then(|d| d.get("chrome"))
        .and_then(|d| d.as_array())
        .ok_or_else(|| anyhow::anyhow!("unexpected Chrome for Testing index layout"))?;
    for entry in downloads {
        if entry.get("platform").and_then(|p| p.as_str()) == Some(key) {
            if let Some(url) = entry.get("url").and_then(|u| u.as_str()) {
                return Ok(url.to_string());
            }
        }
    }
    bail!("Chrome for Testing has no build for platform {key}")
}

/// Extract a zip with the tools every supported platform ships.
fn extract_zip(zip: &Path, dest: &Path) -> Result<()> {
    let mut attempts: Vec<(&str, Vec<String>)> = Vec::new();
    attempts.push(("tar", vec!["-xf".into(), zip.display().to_string(), "-C".into(), dest.display().to_string()]));
    #[cfg(target_os = "windows")]
    attempts.push((
        "powershell",
        vec![
            "-NoProfile".into(),
            "-Command".into(),
            format!(
                "Expand-Archive -LiteralPath '{}' -DestinationPath '{}' -Force",
                zip.display(),
                dest.display()
            ),
        ],
    ));
    #[cfg(not(target_os = "windows"))]
    attempts.push(("unzip", vec!["-q".into(), "-o".into(), zip.display().to_string(), "-d".into(), dest.display().to_string()]));

    let mut last_error = String::new();
    for (program, args) in attempts {
        match std::process::Command::new(program).args(&args).status() {
            Ok(status) if status.success() => return Ok(()),
            Ok(status) => last_error = format!("{program} exited with {status}"),
            Err(e) => last_error = format!("could not run {program}: {e}"),
        }
    }
    bail!("could not extract {}: {last_error}", zip.display())
}

/// Find a Chromium executable in a downloaded tree.
fn find_chrome(dir: &Path) -> Option<PathBuf> {
    let mut fallback: Option<PathBuf> = None;
    let mut stack = vec![(dir.to_path_buf(), 0usize)];
    while let Some((current, depth)) = stack.pop() {
        if depth > 6 {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push((path, depth + 1));
                continue;
            }
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if name.eq_ignore_ascii_case("chrome.exe") || name.eq_ignore_ascii_case("chrome") {
                return Some(path);
            }
            if fallback.is_none()
                && (name.starts_with("Google Chrome") || name.eq_ignore_ascii_case("Chromium"))
            {
                fallback = Some(path);
            }
        }
    }
    #[cfg(unix)]
    if let Some(path) = &fallback {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(meta) = std::fs::metadata(path) {
                let mut perms = meta.permissions();
                perms.set_mode(0o755);
                let _ = std::fs::set_permissions(path, perms);
            }
        }
    }
    fallback
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_key_is_known() {
        assert!(matches!(
            platform_key(),
            "win64" | "mac-arm64" | "mac-x64" | "linux64"
        ));
    }

    #[test]
    fn finds_nested_chrome_executable() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("chrome-win64").join("chrome-win64");
        std::fs::create_dir_all(&nested).unwrap();
        let exe = nested.join("chrome.exe");
        std::fs::write(&exe, b"stub").unwrap();
        assert_eq!(find_chrome(dir.path()), Some(exe));
    }

    #[test]
    fn missing_tree_has_no_executable() {
        let dir = tempfile::tempdir().unwrap();
        assert!(find_chrome(dir.path()).is_none());
    }
}
