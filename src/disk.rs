use crate::console::{p_print, separator, Color};
use crate::util::human_size;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

pub const DEFAULT_BACKUP_KEEP: usize = 100;

pub fn profile_root() -> PathBuf {
    std::env::temp_dir().join("MegaTemp")
}

pub fn backup_dir() -> PathBuf {
    PathBuf::from("credentials").join("pruned")
}

pub fn profile_dir(name: &str) -> PathBuf {
    let dir = profile_root().join(name);
    let _ = std::fs::create_dir_all(&dir);
    dir
}

pub fn dir_size(path: &Path) -> u64 {
    let mut total = 0;
    let Ok(entries) = std::fs::read_dir(path) else {
        return 0;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            total += dir_size(&p);
        } else if let Ok(meta) = entry.metadata() {
            total += meta.len();
        }
    }
    total
}

/// Remove browser profiles older than 24 hours; keep the current run's fresh ones.
pub fn cleanup_stale_profiles(max_age: Duration) -> usize {
    let root = profile_root();
    let Ok(entries) = std::fs::read_dir(&root) else {
        return 0;
    };
    let now = SystemTime::now();
    let mut removed = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let age = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|m| now.duration_since(m).ok())
            .unwrap_or_default();
        if age > max_age && std::fs::remove_dir_all(&path).is_ok() {
            removed += 1;
        }
    }
    removed
}

pub fn cleanup_orphan_parts(root: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(root) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("part")
            && std::fs::remove_file(&path).is_ok()
        {
            removed += 1;
        }
    }
    removed
}

pub fn cleanup_update_leftovers() -> usize {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(_) => return 0,
    };
    let mut removed = 0;
    for ext in ["old", "new"] {
        let leftover = exe.with_extension(ext);
        if leftover.exists() && std::fs::remove_file(&leftover).is_ok() {
            removed += 1;
        }
    }
    removed
}

pub fn prune_backups(keep: usize) -> usize {
    let dir = backup_dir();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return 0;
    };
    let mut files: Vec<(SystemTime, PathBuf)> = entries
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            if p.is_file() {
                Some((e.metadata().and_then(|m| m.modified()).ok()?, p))
            } else {
                None
            }
        })
        .collect();
    if files.len() <= keep {
        return 0;
    }
    files.sort_by(|a, b| b.0.cmp(&a.0));
    let mut removed = 0;
    for (_, path) in files.into_iter().skip(keep) {
        if std::fs::remove_file(path).is_ok() {
            removed += 1;
        }
    }
    removed
}

pub fn usage_report() {
    separator("Disk usage", 60);
    let rows = [
        ("Chromium profiles", profile_root()),
        ("Credentials", PathBuf::from("credentials")),
        ("Pruned backups", backup_dir()),
    ];
    for (label, path) in rows {
        let size = dir_size(&path);
        println!("  {:<24} {:>12}  {}", label, human_size(size), path.display());
    }
    let parts: u64 = std::fs::read_dir(profile_root())
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("part"))
                .filter_map(|e| e.metadata().ok())
                .map(|m| m.len())
                .sum()
        })
        .unwrap_or(0);
    println!("  {:<24} {:>12}", "Orphan .part files", human_size(parts));
    if let Ok(exe) = std::env::current_exe() {
        let size = std::fs::metadata(&exe).map(|m| m.len()).unwrap_or(0);
        println!("  {:<24} {:>12}  {}", "MegaTemp executable", human_size(size), exe.display());
    }
    separator("", 60);
}

pub fn clean_all() {
    separator("Cleaning up", 60);
    clear_profiles();
    let parts = cleanup_orphan_parts(&profile_root());
    let partials = cleanup_orphan_parts(Path::new("."));
    let leftovers = cleanup_update_leftovers();
    let backups = prune_backups(DEFAULT_BACKUP_KEEP);
    p_print(
        &format!(
            "Removed {parts} profile part(s), {partials} download part(s), {leftovers} update leftover(s), {backups} old backup(s)."
        ),
        Color::Ok,
    );
}

pub fn clear_profiles() {
    let root = profile_root();
    if root.exists() {
        if std::fs::remove_dir_all(&root).is_err() {
            crate::console::kill_process(&root.to_string_lossy());
            let _ = std::fs::remove_dir_all(&root);
        }
    }
    let _ = std::fs::create_dir_all(&root);
}
