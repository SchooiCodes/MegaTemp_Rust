use anyhow::{bail, Result};
use std::path::Path;

use crate::console::{p_print, separator, Color};
use crate::mega::MegaNode;
use crate::models::{Config, Credentials};

pub async fn list_files(cfg: &Config, creds: &Credentials) -> Result<Vec<MegaNode>> {
    let client = super::upload::get_session(cfg, creds).await?;
    client.list_root_files().await
}

pub async fn download_file(
    cfg: &Config,
    creds: &Credentials,
    node: &MegaNode,
    dest_dir: &Path,
) -> Result<std::path::PathBuf> {
    if !dest_dir.is_dir() {
        bail!("{} is not a directory", dest_dir.display());
    }
    let client = super::upload::get_session(cfg, creds).await?;
    let name = if node.name.trim().is_empty() {
        format!("mega-download-{}", node.handle)
    } else {
        Path::new(&node.name)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| node.name.clone())
    };
    let dest = dest_dir.join(name);
    let mut last = None;
    for attempt in 1..=3 {
        match client.download(node, &dest).await {
            Ok(()) => {
                p_print(&format!("Downloaded {}", dest.display()), Color::Ok);
                return Ok(dest);
            }
            Err(e) => {
                last = Some(e);
                if attempt < 3 {
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                }
            }
        }
    }
    Err(last.unwrap_or_else(|| anyhow::anyhow!("download failed")))
}

/// Interactive cloud browser used by the menu.
pub async fn browse_cloud(cfg: &Config, creds: &Credentials) -> Result<()> {
    let files = list_files(cfg, creds).await?;
    if files.is_empty() {
        p_print("No files in the cloud drive.", Color::Warning);
        return Ok(());
    }
    separator("Cloud files", 60);
    for (i, file) in files.iter().enumerate() {
        println!(
            "  {:>3}. {:<40} {:>10}",
            i + 1,
            file.name,
            crate::util::human_size(file.size)
        );
    }
    let choice = crate::util::prompt("Select a file number to download (blank to cancel): ");
    let Ok(index) = choice.parse::<usize>() else {
        return Ok(());
    };
    if index == 0 || index > files.len() {
        return Ok(());
    }
    let dest = crate::util::prompt("Destination directory [.]: ");
    let dest = if dest.trim().is_empty() { "." } else { &dest };
    download_file(cfg, creds, &files[index - 1], Path::new(dest)).await?;
    Ok(())
}
