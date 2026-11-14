use anyhow::{bail, Result};
use std::time::Duration;

use crate::console::{p_print, Color};

pub struct Release {
    pub tag: String,
    pub asset_url: Option<String>,
    pub checksum_url: Option<String>,
}

fn asset_name() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        "MegaTemp-windows.exe"
    }
    #[cfg(target_os = "linux")]
    {
        "MegaTemp-linux"
    }
    #[cfg(target_os = "macos")]
    {
        "MegaTemp-macos"
    }
}

pub async fn check_latest(http: &reqwest::Client) -> Result<Release> {
    let resp = http
        .get(crate::console::RELEASES_URL)
        .header("accept", "application/vnd.github+json")
        .send()
        .await?
        .error_for_status()?;
    let value: serde_json::Value = resp.json().await?;
    let release = value
        .as_array()
        .and_then(|a| a.first())
        .ok_or_else(|| anyhow::anyhow!("no releases found"))?;
    let tag = release
        .get("tag_name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let assets = release
        .get("assets")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let find = |name: &str| {
        assets.iter().find_map(|a| {
            let asset_name = a.get("name").and_then(|v| v.as_str())?;
            if asset_name == name {
                a.get("browser_download_url")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            } else {
                None
            }
        })
    };
    let asset_url = find(asset_name());
    let checksum_url = find(&format!("{}.sha256", asset_name()));
    Ok(Release {
        tag,
        asset_url,
        checksum_url,
    })
}

pub async fn run_update(yes: bool) -> Result<bool> {
    let http = reqwest::Client::builder()
        .user_agent("MegaTemp")
        .timeout(Duration::from_secs(60))
        .build()?;
    let release = check_latest(&http).await?;
    let current = crate::console::VERSION;
    if release.tag == current || release.tag.is_empty() {
        p_print("MegaTemp is up to date.", Color::Ok);
        return Ok(false);
    }
    p_print(
        &format!("A newer release is available: {} (current {current})", release.tag),
        Color::Cyan,
    );
    if !yes && !crate::util::prompt_yes_no("Download and install it?", false) {
        return Ok(false);
    }
    let Some(url) = release.asset_url else {
        bail!("no release asset for this platform");
    };
    p_print("Downloading update...", Color::Cyan);
    let bytes = http.get(url).send().await?.error_for_status()?.bytes().await?;
    if let Some(checksum_url) = release.checksum_url {
        if let Ok(resp) = http.get(checksum_url).send().await {
            if let Ok(text) = resp.text().await {
                let expected = text.split_whitespace().next().unwrap_or("").to_lowercase();
                use sha2::Digest;
                let actual = format!("{:x}", sha2::Sha256::digest(&bytes));
                if !expected.is_empty() && expected != actual {
                    bail!("downloaded update failed checksum verification");
                }
            }
        }
    }
    let exe = std::env::current_exe()?;
    let old = exe.with_extension("old");
    let _ = std::fs::remove_file(&old);
    std::fs::rename(&exe, &old)?;
    std::fs::write(&exe, &bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&exe)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&exe, perms)?;
    }
    p_print(
        &format!("Updated to {}. Restart MegaTemp to use it.", release.tag),
        Color::Ok,
    );
    Ok(true)
}

pub async fn auto_update() {
    // The Python implementation only updates frozen builds; keep that behavior.
    let http = match reqwest::Client::builder()
        .user_agent("MegaTemp")
        .timeout(Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(_) => return,
    };
    if let Ok(release) = check_latest(&http).await {
        if !release.tag.is_empty() && release.tag != crate::console::VERSION {
            p_print(
                &format!(
                    "A new release is available: {} (run with --update to install)",
                    release.tag
                ),
                Color::Cyan,
            );
        }
    }
}
