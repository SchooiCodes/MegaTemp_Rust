use anyhow::{bail, Result};
use std::path::Path;
use std::time::{Duration, Instant};

use crate::console::{elapsed, p_print, Color};
use crate::mega::{MegaClient, UploadResult};
use crate::models::{Config, Credentials};

/// Show `Uploading name... (Ns elapsed)` on the live status line while the
/// upload runs, mirroring Python's `_upload_with_progress`.
fn spawn_upload_progress(name: String) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let started = Instant::now();
        let mut ticker = tokio::time::interval(Duration::from_millis(500));
        loop {
            ticker.tick().await;
            crate::console::status_line(&format!(
                "Uploading {}... ({}s elapsed)",
                name,
                started.elapsed().as_secs()
            ));
        }
    })
}

pub async fn get_session(cfg: &Config, creds: &Credentials) -> Result<MegaClient> {
    if creds.email.is_empty() || creds.password.is_empty() {
        bail!("account is missing email or password");
    }
    let proxy = if cfg.proxy.is_empty() {
        None
    } else {
        Some(cfg.proxy.as_str())
    };
    let mut last = None;
    for _ in 0..3 {
        let mut client = MegaClient::new(proxy)?;
        match client.login(&creds.email, &creds.password).await {
            Ok(()) => return Ok(client),
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or_else(|| anyhow::anyhow!("login failed")))
}

pub async fn upload_file(
    cfg: &Config,
    creds: &Credentials,
    path: &Path,
    public: bool,
) -> Result<UploadResult> {
    if !path.is_file() {
        bail!("{} is not a file", path.display());
    }
    let client = get_session(cfg, creds).await?;
    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    p_print(
        &format!(
            "Uploading {} ({:.2} MiB)",
            path.display(),
            size as f64 / 1_048_576.0
        ),
        Color::Cyan,
    );
    let start = Instant::now();
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string());
    let mut last = None;
    for attempt in 1..=2 {
        let progress = spawn_upload_progress(name.clone());
        let outcome = client.upload(path, None, None, public).await;
        progress.abort();
        crate::console::clear_status_line();
        match outcome {
            Ok(result) => {
                p_print(
                    &format!("Uploaded in {}", elapsed(start)),
                    Color::Ok,
                );
                if let Some(url) = &result.public_url {
                    p_print(url, Color::Blue);
                }
                return Ok(result);
            }
            Err(e) => {
                last = Some(e);
                if attempt < 2 {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
            }
        }
    }
    Err(last.unwrap_or_else(|| anyhow::anyhow!("upload failed")))
}

pub async fn upload_dir(
    cfg: &Config,
    creds: &Credentials,
    dir: &Path,
    public: bool,
) -> Result<usize> {
    if !dir.is_dir() {
        bail!("{} is not a directory", dir.display());
    }
    let client = get_session(cfg, creds).await?;
    let mut files = Vec::new();
    collect_files(dir, &mut files);
    for file in &files {
        let relative = file.strip_prefix(dir).unwrap_or(file);
        let target_folder = relative.parent().filter(|p| !p.as_os_str().is_empty());
        let dest = if let Some(parent) = target_folder {
            Some(
                client
                    .create_folder(&parent.to_string_lossy(), None)
                    .await?
                    .handle,
            )
        } else {
            None
        };
        let name = file
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let progress = spawn_upload_progress(name.clone());
        let outcome = client
            .upload(file, dest.as_deref(), Some(&name), public)
            .await;
        progress.abort();
        crate::console::clear_status_line();
        match outcome {
            Ok(result) => {
                p_print(&format!("Uploaded {}", file.display()), Color::Ok);
                if let Some(url) = result.public_url {
                    p_print(&url, Color::Blue);
                }
            }
            Err(e) => p_print(
                &format!("Failed to upload {}: {e}", file.display()),
                Color::Fail,
            ),
        }
    }
    Ok(files.len())
}

fn collect_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, out);
        } else if path.is_file() {
            out.push(path);
        }
    }
}
