use anyhow::{bail, Result};
use fs2::FileExt;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Advisory lock stored next to a data file as `<path>.lock`.
/// The lock file is never deleted (matching the Python implementation).
pub struct FileLock {
    file: File,
    path: PathBuf,
}

impl FileLock {
    pub fn acquire(path: &Path, timeout: Duration) -> Result<Self> {
        let lock_path = PathBuf::from(format!("{}.lock", path.display()));
        if let Some(parent) = lock_path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&lock_path)?;

        let deadline = Instant::now() + timeout;
        loop {
            match file.try_lock_exclusive() {
                Ok(()) => {
                    return Ok(Self {
                        file,
                        path: lock_path,
                    })
                }
                Err(_) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(e) => bail!("could not lock {}: {e}", lock_path.display()),
            }
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

pub fn acquire(path: &Path) -> Result<FileLock> {
    FileLock::acquire(path, Duration::from_secs(30))
}
