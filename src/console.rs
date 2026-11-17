use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

pub const VERSION: &str = "v1.8.0";
pub const RELEASES_URL: &str = "https://api.github.com/repos/SchooiCodes/MegaTemp/releases";

/// A structured progress event.
///
/// The terminal ignores these; a GUI observer turns them into cards, tables
/// and toasts instead of parsing log text.
#[derive(Debug, Clone)]
pub enum Event {
    RunStarted {
        kind: String,
        total: u64,
        workers: usize,
    },
    AccountCreated {
        worker: Option<String>,
        email: String,
        password: String,
        seconds: f64,
    },
    RunProgress {
        done: u64,
        total: u64,
        ok: u64,
        failed: u64,
    },
    RunFinished {
        ok: u64,
        failed: u64,
        seconds: f64,
    },
    KeepaliveAccount {
        email: String,
        alive: bool,
        quota_gb: Option<f64>,
        error: Option<String>,
    },
    KeepaliveFinished {
        total: usize,
        alive: usize,
        dead: usize,
        pruned: usize,
    },
    Toast {
        kind: String,
        text: String,
    },
}

/// One piece of output handed to an observer.
#[derive(Debug, Clone)]
pub enum Output {
    Line {
        text: String,
        colour: Color,
        worker: Option<String>,
    },
    Status(String),
    Event(Event),
}

/// Receiver for every line, status update and event the tool produces.
pub type OutputHook = Arc<dyn Fn(Output) + Send + Sync>;

static HOOK: OnceLock<Mutex<Option<OutputHook>>> = OnceLock::new();

fn hook_slot() -> &'static Mutex<Option<OutputHook>> {
    HOOK.get_or_init(|| Mutex::new(None))
}

/// Install (or clear, with `None`) the observer used by the GUI.
pub fn set_output_hook(hook: Option<OutputHook>) {
    *hook_slot().lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = hook;
}

fn dispatch(output: Output) {
    let hook = hook_slot()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    if let Some(hook) = hook {
        hook(output);
    }
}

/// Report a structured progress event to an attached observer.
pub fn event(event: Event) {
    dispatch(Output::Event(event));
}

static QUIET: AtomicBool = AtomicBool::new(false);
static JSON_MODE: AtomicBool = AtomicBool::new(false);
static NOTIFY: AtomicBool = AtomicBool::new(false);
static VERBOSE: AtomicBool = AtomicBool::new(false);
static STATUS_LAST: AtomicI64 = AtomicI64::new(0);
/// Serialise every write so a colour escape and its text are never split by
/// another worker, and a live status line is always erased before a new line.
static WRITE_LOCK: Mutex<()> = Mutex::new(());
/// True while a live (non-newline) status line is showing on a terminal.
static STATUS_ACTIVE: AtomicBool = AtomicBool::new(false);

tokio::task_local! {
    /// Output tag for the current task, for example `w2`.
    ///
    /// Task-locals travel with the task when tokio moves it between threads,
    /// unlike thread-locals, so a parallel worker keeps its tag.
    static LABEL: Option<String>;
    /// Lines captured for the current task, printed together when it finishes.
    static TASK_CAPTURE: RefCell<Option<Vec<CapturedLine>>>;
}

/// A buffered line kept in both its coloured and plain forms.
#[derive(Debug, Clone)]
struct CapturedLine {
    rendered: String,
    plain: String,
    colour: Color,
}

/// Run `future` with every line it prints prefixed by `[label]`.
///
/// Wrap a unit of work in [`collected`] as well to print its lines as one
/// block; the label alone only tags them.
pub fn labelled<F>(
    label: impl Into<String>,
    future: F,
) -> impl std::future::Future<Output = F::Output>
where
    F: std::future::Future,
{
    LABEL.scope(Some(label.into()), future)
}

/// Run `future`, buffering everything it prints and writing the lines as one
/// block when it finishes.
///
/// Parallel workers wrap each registration in this so concurrent accounts never
/// interleave line by line, while still streaming progress per account.
pub async fn collected<F>(future: F) -> F::Output
where
    F: std::future::Future,
{
    TASK_CAPTURE
        .scope(RefCell::new(Some(Vec::new())), async move {
            let out = future.await;
            flush_task_capture();
            out
        })
        .await
}

/// Write every line captured by the current task's buffer.
fn flush_task_capture() {
    let lines = TASK_CAPTURE
        .try_with(|cell| cell.borrow_mut().take().unwrap_or_default())
        .unwrap_or_default();
    if lines.is_empty() {
        return;
    }
    let worker = current_label();
    {
        let _guard = lock_writes();
        let tty = std::io::stdout().is_terminal();
        let mut out = std::io::stdout().lock();
        for line in &lines {
            end_status_line(&mut out, tty);
            let _ = writeln!(out, "{}", line.rendered);
        }
        let _ = out.flush();
    }
    for line in lines {
        dispatch(Output::Line {
            text: line.plain,
            colour: line.colour,
            worker: worker.clone(),
        });
    }
}

/// Tag of the current task, or `None` for single, untagged runs.
pub fn current_label() -> Option<String> {
    LABEL.try_with(|label| label.clone()).ok().flatten()
}

/// Plain (uncoloured) `[w1] ` style prefix for observers.
fn label_plain() -> String {
    match current_label() {
        Some(label) => format!("[{label}] "),
        None => String::new(),
    }
}

fn label_prefix() -> String {    match current_label() {
        Some(label) => {
            if colors_enabled() {
                // Give each worker its own colour so concurrent lines are easy to
                // follow at a glance.
                let palette = ["\x1b[96m", "\x1b[95m", "\x1b[93m", "\x1b[94m", "\x1b[92m"];
                let idx = label.bytes().map(usize::from).sum::<usize>() % palette.len();
                format!("{}[{label}]\x1b[0m ", palette[idx])
            } else {
                format!("[{label}] ")
            }
        }
        None => String::new(),
    }
}

fn lock_writes() -> std::sync::MutexGuard<'static, ()> {
    WRITE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Erase the live status line, if one is showing, before printing a new line.
fn end_status_line(out: &mut impl Write, tty: bool) {
    if tty && STATUS_ACTIVE.swap(false, Ordering::Relaxed) {
        let _ = write!(out, "\r\x1b[K");
    }
}

thread_local! {
    static CAPTURE: RefCell<Option<Vec<CapturedLine>>> = const { RefCell::new(None) };
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
pub fn set_verbose(v: bool) {
    VERBOSE.store(v, Ordering::Relaxed);
}
pub fn verbose() -> bool {
    VERBOSE.load(Ordering::Relaxed)
}
/// Print only when `--verbose` is on.
pub fn v_print(text: &str, colour: Color) {
    if verbose() {
        p_print(text, colour);
    }
}

/// Print human output. In JSON mode it goes to stderr, keeping stdout clean.
pub fn p_print(text: &str, colour: Color) {
    if quiet() && !matches!(colour, Color::Warning | Color::Fail) {
        return;
    }
    let prefix = label_prefix();
    let plain = format!("{}{}", label_plain(), text);
    let rendered = if colors_enabled() {
        let c = code(colour);
        format!("{c}{prefix}{c}{text}{}", code(Color::End))
    } else {
        format!("{prefix}{text}")
    };

    // A worker task buffers its own lines until it finishes, so concurrent
    // accounts never interleave mid-block. Synchronous captures still work for
    // callers that opt into them explicitly.
    let entry = CapturedLine {
        rendered: rendered.clone(),
        plain: plain.clone(),
        colour,
    };
    let buffered = TASK_CAPTURE.try_with(|cell| {
        let mut slot = cell.borrow_mut();
        match slot.as_mut() {
            Some(buf) => {
                buf.push(entry.clone());
                true
            }
            None => false,
        }
    });
    let buffered = match buffered {
        Ok(true) => true,
        Ok(false) | Err(_) => CAPTURE.with(|cell| {
            if let Some(buf) = cell.borrow_mut().as_mut() {
                buf.push(entry);
                true
            } else {
                false
            }
        }),
    };
    if buffered {
        return;
    }
    {
        let _guard = lock_writes();
        if JSON_MODE.load(Ordering::Relaxed) {
            eprintln!("{rendered}");
        } else {
            let tty = std::io::stdout().is_terminal();
            let mut out = std::io::stdout().lock();
            end_status_line(&mut out, tty);
            let _ = writeln!(out, "{rendered}");
            let _ = out.flush();
        }
    }
    dispatch(Output::Line {
        text: plain,
        colour,
        worker: current_label(),
    });
}

pub fn capture_start() {
    CAPTURE.with(|cell| *cell.borrow_mut() = Some(Vec::new()));
}

pub fn capture_end() -> Vec<String> {
    CAPTURE.with(|cell| {
        cell.borrow_mut()
            .take()
            .unwrap_or_default()
            .into_iter()
            .map(|line| line.rendered)
            .collect()
    })
}

pub fn separator(title: &str, width: usize) {
    let ch = if unicode_enabled() { "─" } else { "-" };
    let line = if title.is_empty() {
        ch.repeat(width)
    } else {
        let label = format!(" {title} ");
        let visible = label.chars().count();
        if visible >= width {
            label
        } else {
            let left = (width - visible) / 2;
            let right = width - visible - left;
            format!("{}{}{}", ch.repeat(left), label, ch.repeat(right))
        }
    };
    let captured = TASK_CAPTURE
        .try_with(|cell| {
            let mut slot = cell.borrow_mut();
            if let Some(buf) = slot.as_mut() {
                buf.push(CapturedLine {
                    rendered: line.clone(),
                    plain: line.clone(),
                    colour: Color::Header,
                });
                true
            } else {
                false
            }
        })
        .unwrap_or(false);
    if captured {
        return;
    }
    {
        let _guard = lock_writes();
        let tty = std::io::stdout().is_terminal();
        let mut out = std::io::stdout().lock();
        end_status_line(&mut out, tty);
        let _ = writeln!(out, "{line}");
        let _ = out.flush();
    }
    dispatch(Output::Line {
        text: line,
        colour: Color::Header,
        worker: current_label(),
    });
}

pub fn elapsed(start: Instant) -> String {
    let secs = start.elapsed().as_secs_f64();
    if secs < 60.0 {
        format!("{secs:.1}s")
    } else {
        format!("{:.0}m {:.1}s", (secs / 60.0).floor(), secs % 60.0)
    }
}

/// True when the current task is collecting its output for a grouped flush.
fn capturing() -> bool {
    TASK_CAPTURE
        .try_with(|cell| cell.borrow().is_some())
        .unwrap_or(false)
}

pub fn status_line(text: &str) {
    // A captured block is printed later, so a redirected status line would
    // land before the lines that logically precede it. On a terminal the
    // single live line is erased before the block appears, so it stays.
    if capturing() && !std::io::stdout().is_terminal() {
        return;
    }
    let now = chrono::Utc::now().timestamp_millis();
    let tty = std::io::stdout().is_terminal();
    if !tty {
        let last = STATUS_LAST.load(Ordering::Relaxed);
        if now - last < 3000 {
            return;
        }
    }
    STATUS_LAST.store(now, Ordering::Relaxed);
    let rendered = format!("{}{}", label_prefix(), text);
    dispatch(Output::Status(format!("{}{}", label_plain(), text)));
    let _guard = lock_writes();
    if tty {
        let mut out = std::io::stdout().lock();
        let _ = write!(out, "\r\x1b[K{rendered}");
        let _ = out.flush();
        STATUS_ACTIVE.store(true, Ordering::Relaxed);
    } else if json_mode() {
        eprintln!("{rendered}");
    } else {
        // Redirected output has no cursor to rewrite, so emit whole lines.
        // Printing without the newline merged status text into the next line.
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{rendered}");
        let _ = out.flush();
    }
}

pub fn clear_status_line() {
    if !std::io::stdout().is_terminal() || !STATUS_ACTIVE.load(Ordering::Relaxed) {
        return;
    }
    let _guard = lock_writes();
    let mut out = std::io::stdout().lock();
    end_status_line(&mut out, true);
    let _ = out.flush();
}

pub fn clear_console() {
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
