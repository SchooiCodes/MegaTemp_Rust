use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::time::{Duration, Instant};

pub const VERSION: &str = "v1.8.0";
pub const RELEASES_URL: &str = "https://api.github.com/repos/SchooiCodes/MegaTemp/releases";

static QUIET: AtomicBool = AtomicBool::new(false);
static JSON_MODE: AtomicBool = AtomicBool::new(false);
static NOTIFY: AtomicBool = AtomicBool::new(false);
static STATUS_LAST: AtomicI64 = AtomicI64::new(0);

thread_local! {
    static CAPTURE: RefCell<Option<Vec<String>>> = const { RefCell::new(None) };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    Header,
    Blue,
    Cyan,
    Green,
    Warning,
    Fail,
    Ok,
    End,
    None,
}

fn code(c: Color) -> &'static str {
    match c {
        Color::Header => "\x1b[95m",
        Color::Blue => "\x1b[94m",
        Color::Cyan => "\x1b[96m",
        Color::Green => "\x1b[92m",
        Color::Warning => "\x1b[93m",
        Color::Fail => "\x1b[91m",
        Color::Ok => "\x1b[92m",
        Color::End => "\x1b[0m",
        Color::None => "",
    }
}

pub fn colors_enabled() -> bool {
    if std::env::var_os("NO_COLOR").is_some() {
        return false;
    }
    if std::env::var("CLICOLOR").map(|v| v == "0").unwrap_or(false) {
        return false;
    }
    if std::env::var("FORCE_COLOR").is_ok() {
        return true;
    }
    if std::env::var("TERM").map(|v| v == "dumb").unwrap_or(false) {
        return false;
    }
    use std::io::IsTerminal;
    std::io::stdout().is_terminal()
}

pub fn unicode_enabled() -> bool {
    if std::env::var("MEGATEMP_ASCII").is_ok() {
        return false;
    }
    if std::env::var("TERM").map(|v| v == "dumb").unwrap_or(false) {
        return false;
    }
    true
}

pub fn col(c: Color) -> &'static str {
    if colors_enabled() {
        code(c)
    } else {
        ""
    }
}

pub fn set_quiet(v: bool) {
    QUIET.store(v, Ordering::Relaxed);
}
pub fn quiet() -> bool {
    QUIET.load(Ordering::Relaxed)
}
pub fn set_json(v: bool) {
    JSON_MODE.store(v, Ordering::Relaxed);
}
pub fn json_mode() -> bool {
    JSON_MODE.load(Ordering::Relaxed)
}
pub fn set_notify(v: bool) {
    NOTIFY.store(v, Ordering::Relaxed);
}
pub fn notify_enabled() -> bool {
    NOTIFY.load(Ordering::Relaxed)
}

/// Print human output. In JSON mode it goes to stderr, keeping stdout clean.
pub fn p_print(text: &str, colour: Color) {
    if quiet() && !matches!(colour, Color::Warning | Color::Fail) {
        return;
    }
    let rendered = if colors_enabled() {
        format!("{}{}{}", code(colour), text, code(Color::End))
    } else {
        text.to_string()
    };

    let buffered = CAPTURE.with(|cell| {
        if let Some(buf) = cell.borrow_mut().as_mut() {
            buf.push(rendered.clone());
            true
        } else {
            false
        }
    });
    if buffered {
        return;
    }
    if JSON_MODE.load(Ordering::Relaxed) {
        eprintln!("{rendered}");
    } else {
        println!("{rendered}");
    }
}

pub fn capture_start() {
    CAPTURE.with(|cell| *cell.borrow_mut() = Some(Vec::new()));
}

pub fn capture_end() -> Vec<String> {
    CAPTURE.with(|cell| cell.borrow_mut().take().unwrap_or_default())
}

pub fn separator(title: &str, width: usize) {
    let ch = if unicode_enabled() { "─" } else { "-" };
    if title.is_empty() {
        println!("{}", ch.repeat(width));
        return;
    }
    let label = format!(" {title} ");
    let visible = label.chars().count();
    if visible >= width {
        println!("{label}");
        return;
    }
    let left = (width - visible) / 2;
    let right = width - visible - left;
    println!("{}{}{}", ch.repeat(left), label, ch.repeat(right));
}

pub fn elapsed(start: Instant) -> String {
    let secs = start.elapsed().as_secs_f64();
    if secs < 60.0 {
        format!("{secs:.1}s")
    } else {
        format!("{:.0}m {:.1}s", (secs / 60.0).floor(), secs % 60.0)
    }
}

pub fn status_line(text: &str) {
    use std::io::IsTerminal;
    let now = chrono::Utc::now().timestamp_millis();
    if !std::io::stdout().is_terminal() {
        let last = STATUS_LAST.load(Ordering::Relaxed);
        if now - last < 3000 {
            return;
        }
    }
    STATUS_LAST.store(now, Ordering::Relaxed);
    print!("\r\x1b[K{text}");
    let _ = std::io::stdout().flush();
}

pub fn clear_status_line() {
    use std::io::IsTerminal;
    if std::io::stdout().is_terminal() {
        print!("\r\x1b[K");
        let _ = std::io::stdout().flush();
    }
}

pub fn clear_console() {
    use std::io::IsTerminal;
    if std::io::stdout().is_terminal() {
        print!("\x1b[2J\x1b[H");
        let _ = std::io::stdout().flush();
    }
}

/// POST `{"event":..., "data":...}` to a webhook, silently ignoring failures.
pub fn send_webhook(url: &str, event: &str, data: serde_json::Value) {
    if url.is_empty() || quiet() {
        return;
    }
    let url = url.to_string();
    let event = event.to_string();
    std::thread::spawn(move || {
        let client = match reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
        {
            Ok(c) => c,
            Err(_) => return,
        };
        let payload = serde_json::json!({ "event": event, "data": data });
        let _ = client.post(&url).json(&payload).send();
    });
}

pub fn notify(title: &str, message: &str) {
    if !notify_enabled() {
        return;
    }
    #[cfg(target_os = "windows")]
    {
        let _ = (title, message);
    }
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("notify-send").arg(title).arg(message).spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let script = format!(
            "display notification \"{}\" with title \"{}\"",
            message.replace('"', "\\\""),
            title.replace('"', "\\\"")
        );
        let _ = std::process::Command::new("osascript").arg("-e").arg(script).spawn();
    }
}

pub fn profile_root() -> PathBuf {
    std::env::temp_dir().join("MegaTemp")
}

/// Best-effort cleanup of temporary Chromium profiles.
pub fn clear_tmp() {
    let root = profile_root();
    if !root.exists() {
        return;
    }
    if std::fs::remove_dir_all(&root).is_err() {
        kill_chromium_in(&root);
        std::thread::sleep(Duration::from_millis(300));
        let _ = std::fs::remove_dir_all(&root);
    }
}

fn kill_chromium_in(root: &std::path::Path) {
    use sysinfo::{ProcessesToUpdate, System};
    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::All, true);
    let needle = root.to_string_lossy().to_lowercase();
    for proc in sys.processes().values() {
        let cmd = proc
            .cmd()
            .iter()
            .map(|s| s.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        if cmd.contains(&needle) && cmd.contains("chrom") {
            proc.kill();
        }
    }
}

pub fn kill_process(matches: &str) {
    use sysinfo::{ProcessesToUpdate, System};
    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::All, true);
    let needle = matches.to_lowercase();
    for proc in sys.processes().values() {
        let cmd = proc
            .cmd()
            .iter()
            .map(|s| s.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        if cmd.contains(&needle) {
            proc.kill();
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LoopState {
    #[serde(default)]
    pub total: u64,
    #[serde(default)]
    pub completed: u64,
    #[serde(default)]
    pub failed: u64,
    #[serde(default)]
    pub started_at: f64,
}

pub fn checkpoint_path() -> PathBuf {
    PathBuf::from("loop_state.json")
}

pub fn save_checkpoint(state: &LoopState) -> Result<()> {
    std::fs::write(checkpoint_path(), serde_json::to_string_pretty(state)?)?;
    Ok(())
}

pub fn load_checkpoint() -> Option<LoopState> {
    let text = std::fs::read_to_string(checkpoint_path()).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn clear_checkpoint() {
    let _ = std::fs::remove_file(checkpoint_path());
}
