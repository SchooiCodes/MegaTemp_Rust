use rand::distributions::Alphanumeric;
use rand::Rng;

pub fn mask(value: &str) -> String {
    if value.len() <= 2 {
        "*".repeat(value.len())
    } else {
        format!("{}{}", "*".repeat(value.len() - 2), &value[value.len() - 2..])
    }
}

pub fn random_alnum(n: usize) -> String {
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(n)
        .map(char::from)
        .collect()
}

pub const FIRST_NAMES: &[&str] = &[
    "James", "Mary", "Robert", "Patricia", "John", "Jennifer", "Michael", "Linda", "David",
    "Elizabeth", "William", "Barbara", "Richard", "Susan", "Joseph", "Jessica", "Thomas",
    "Sarah", "Charles", "Karen", "Christopher", "Nancy", "Daniel", "Lisa", "Matthew",
    "Betty", "Anthony", "Margaret", "Mark", "Sandra", "Donald", "Ashley",
];

pub fn random_first_name() -> &'static str {
    FIRST_NAMES[rand::thread_rng().gen_range(0..FIRST_NAMES.len())]
}

/// Expand a leading `~` the way Python's `os.path.expanduser` does.
pub fn expand_user(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\")) {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest).to_string_lossy().to_string();
        }
    } else if path == "~" {
        if let Some(home) = dirs::home_dir() {
            return home.to_string_lossy().to_string();
        }
    }
    path.to_string()
}

/// A 20 GB free-tier storage bar, matching Python's `_storage_bar`.
pub fn storage_bar(quota_gb: f64, width: usize) -> String {
    let ratio = if quota_gb.is_finite() {
        (quota_gb.max(0.0) / 20.0).min(1.0)
    } else {
        0.0
    };
    let filled = (ratio * width as f64) as usize;
    if crate::console::unicode_enabled() {
        format!("{}{}", "█".repeat(filled), "░".repeat(width - filled))
    } else {
        format!("{}{}", "#".repeat(filled), "-".repeat(width - filled))
    }
}

pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Copy text to the system clipboard, mirroring Python's pyperclip usage.
/// Returns false when no clipboard tool is available.
pub fn copy_to_clipboard(text: &str) -> bool {
    use std::io::Write;
    use std::process::{Command, Stdio};

    #[cfg(target_os = "windows")]
    let mut command = Command::new("clip");
    #[cfg(target_os = "macos")]
    let mut command = Command::new("pbcopy");
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        let mut command = if std::env::var_os("WAYLAND_DISPLAY").is_some() {
            Command::new("wl-copy")
        } else {
            let mut xclip = Command::new("xclip");
            xclip.arg("-selection").arg("clipboard");
            xclip
        };
        command
    };

    let Ok(mut child) = command.stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() else {
        return false;
    };
    if let Some(mut stdin) = child.stdin.take() {
        if stdin.write_all(text.as_bytes()).is_err() {
            let _ = child.wait();
            return false;
        }
    }
    child.wait().map(|s| s.success()).unwrap_or(false)
}

pub fn prompt(label: &str) -> String {
    use std::io::Write;
    print!("{label}");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    let _ = std::io::stdin().read_line(&mut line);
    line.trim_end_matches(['\r', '\n']).to_string()
}

pub fn prompt_yes_no(label: &str, default_yes: bool) -> bool {
    let suffix = if default_yes { " [Y/n]: " } else { " [y/N]: " };
    loop {
        let answer = prompt(&format!("{label}{suffix}")).to_lowercase();
        if answer.is_empty() {
            return default_yes;
        }
        match answer.as_str() {
            "y" | "yes" => return true,
            "n" | "no" => return false,
            _ => println!("Please answer yes or no."),
        }
    }
}
