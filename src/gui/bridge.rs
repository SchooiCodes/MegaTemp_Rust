//! Background command channel between the GUI and the async domain layer.
//!
//! The UI thread never blocks: it sends a [`Command`] and drains [`UiEvent`]s
//! once per frame, so registrations and keepalive runs keep the window smooth.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::Instant;

use crate::browser::register::{register_with_retries, resolve_browser, RegisterOptions};
use crate::console::{self, Color, Event, Output};
use crate::models::Config;
use crate::services::account;
use crate::services::download;
use crate::services::upload;
use crate::services::alive::{self, AliveResult, KeepaliveSummary};

/// Work requested by the UI.
#[derive(Debug)]
pub enum Command {
    /// Start a batch with the configuration the user chose in the GUI.
    Create { total: u64, workers: usize, cfg: Config },
    Keepalive { prune: bool },
    UpdateConfig(Config),
    ListCloud { email: String },
    UploadFile { path: std::path::PathBuf, email: String },
    DownloadFile { handle: String, dest: std::path::PathBuf, email: String },
    DeleteAccount { email: String },
    CreateFolder { email: String, name: String },
    InstallChromium,
}

#[derive(Debug, Clone)]
pub struct CloudFile {
    pub handle: String,
    pub name: String,
    pub size: u64,
}

impl From<crate::mega::MegaNode> for CloudFile {
    fn from(node: crate::mega::MegaNode) -> Self {
        Self { handle: node.handle, name: node.name, size: node.size }
    }
}

/// Something the background runtime wants the UI to show.
#[derive(Debug, Clone)]
pub enum UiEvent {
    Line { text: String, colour: Color, worker: Option<String> },
    Status(String),
    Progress { done: u64, total: u64, ok: u64, failed: u64 },
    Created { email: String, password: String, seconds: f64 },
    Finished { ok: u64, failed: u64, seconds: f64 },
    KeepaliveRow { email: String, alive: bool, quota_gb: Option<f64>, error: Option<String> },
    KeepaliveResults(Vec<AliveResult>, KeepaliveSummary),
    CloudFiles { email: String, files: Vec<CloudFile> },
    Toast { kind: String, text: String },
    Idle,
}

pub struct Bridge {
    pub commands: Sender<Command>,
    pub events: Receiver<UiEvent>,
}

impl Bridge {
    pub fn start(cfg: Config) -> Self {
        let (command_tx, command_rx) = mpsc::channel::<Command>();
        let (event_tx, event_rx) = mpsc::channel::<UiEvent>();
        std::thread::spawn(move || {
            let runtime = match tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(e) => {
                    let _ = event_tx.send(UiEvent::Toast {
                        kind: "error".to_string(),
                        text: format!("could not start the background runtime: {e}"),
                    });
                    return;
                }
            };
            attach_hook(event_tx.clone());
            runtime.block_on(async move {
                let mut cfg = cfg;
                while let Ok(command) = command_rx.recv() {
                    match command {
                        Command::Create { total, workers, cfg: run_cfg } => {
                            create_batch(&run_cfg, total, workers, &event_tx).await;
                        }
            Command::Keepalive { prune } => {
                keepalive(&cfg, prune, &event_tx).await;
            }
            Command::UpdateConfig(new) => {
                cfg = new;
            }
            Command::ListCloud { email } => {
                match cred_for(&cfg, &email) {
                    Some(creds) => match download::list_files(&cfg, &creds).await {
                        Ok(files) => { let _ = event_tx.send(UiEvent::CloudFiles { email: email.clone(), files: files.into_iter().map(CloudFile::from).collect() }); }
                        Err(e) => { let _ = event_tx.send(UiEvent::Toast { kind: "error".to_string(), text: format!("{e:#}") }); }
                    },
                    None => { let _ = event_tx.send(UiEvent::Toast { kind: "error".to_string(), text: format!("Could not find saved account {email}") }); }
                }
            }
            Command::UploadFile { path, email } => {
                match cred_for(&cfg, &email) {
                    Some(creds) => match upload::upload_file(&cfg, &creds, &path, false).await {
                        Ok(result) => {
                            let _ = event_tx.send(UiEvent::Toast { kind: "success".to_string(), text: format!("Uploaded {} to {email}", result.handle) });
                            if let Ok(files) = download::list_files(&cfg, &creds).await { let _ = event_tx.send(UiEvent::CloudFiles { email: email.clone(), files: files.into_iter().map(CloudFile::from).collect() }); }
                        }
                        Err(e) => { let _ = event_tx.send(UiEvent::Toast { kind: "error".to_string(), text: format!("{e:#}") }); }
                    },
                    None => { let _ = event_tx.send(UiEvent::Toast { kind: "error".to_string(), text: format!("Could not find saved account {email}") }); }
                }
            }
            Command::DownloadFile { handle, dest, email } => {
                if let Some(creds) = cred_for(&cfg, &email) {
                    if let Ok(files) = download::list_files(&cfg, &creds).await {
                        if let Some(node) = files.into_iter().find(|f| f.handle == handle) {
                            match download::download_file(&cfg, &creds, &node, &dest).await {
                                Ok(path) => { let _ = event_tx.send(UiEvent::Toast { kind: "success".to_string(), text: format!("Downloaded to {}", path.display()) }); }
                                Err(e) => { let _ = event_tx.send(UiEvent::Toast { kind: "error".to_string(), text: format!("{e:#}") }); }
                            }
                        } else {
                            let _ = event_tx.send(UiEvent::Toast { kind: "error".to_string(), text: format!("File {handle} was not found in {email}") });
                        }
                    }
                }
            }
            Command::DeleteAccount { email } => {
                let creds = crate::credentials::load_all(&cfg.encryption_password, &cfg.account_format).into_iter().find(|c| c.creds.email == email && !c.decrypt_failed).map(|c| c.creds);
                if let Some(creds) = creds {
                    match account::delete_account(&cfg, &creds).await {
                        Ok(()) => { let _ = event_tx.send(UiEvent::Toast { kind: "success".to_string(), text: format!("Deleted {email}") }); }
                        Err(e) => { let _ = event_tx.send(UiEvent::Toast { kind: "error".to_string(), text: format!("{e:#}") }); }
                    }
                }
            }
            Command::CreateFolder { email, name } => {
                let creds = crate::credentials::load_all(&cfg.encryption_password, &cfg.account_format).into_iter().find(|c| c.creds.email == email && !c.decrypt_failed).map(|c| c.creds);
                if let Some(creds) = creds {
                    match account::create_folder(&cfg, &creds, &name).await {
                        Ok(_) => { let _ = event_tx.send(UiEvent::Toast { kind: "success".to_string(), text: format!("Created folder {name} in {email}") }); }
                        Err(e) => { let _ = event_tx.send(UiEvent::Toast { kind: "error".to_string(), text: format!("{e:#}") }); }
                    }
                }
            }
            Command::InstallChromium => {
                match crate::browser::install::download_chromium().await {
                    Ok(path) => {
                        cfg.executable_path = path.to_string_lossy().to_string();
                        let _ = crate::config::save(None, &cfg);
                        let _ = event_tx.send(UiEvent::Toast { kind: "success".to_string(), text: format!("Chromium installed at {}", path.display()) });
                    }
                    Err(e) => { let _ = event_tx.send(UiEvent::Toast { kind: "error".to_string(), text: format!("{e:#}") }); }
                }
            }
        }
                    let _ = event_tx.send(UiEvent::Idle);
                }
            });
        });
        Bridge {
            commands: command_tx,
            events: event_rx,
        }
    }
}

fn attach_hook(tx: Sender<UiEvent>) {
    console::set_output_hook(Some(Arc::new(move |output: Output| {
        let event = match output {
            Output::Line {
                text,
                colour,
                worker,
            } => UiEvent::Line {
                text,
                colour,
                worker,
            },
            Output::Status(text) => UiEvent::Status(text),
            Output::Event(Event::AccountCreated {
                email,
                password,
                seconds,
                ..
            }) => UiEvent::Created {
                email,
                password,
                seconds,
            },
            Output::Event(Event::KeepaliveAccount {
                email,
                alive,
                quota_gb,
                error,
            }) => UiEvent::KeepaliveRow {
                email,
                alive,
                quota_gb,
                error,
            },
            Output::Event(Event::Toast { kind, text }) => UiEvent::Toast { kind, text },
            Output::Event(_) => return,
        };
        let _ = tx.send(event);
    })));
}

/// Resolve the account selected in the Cloud screen. An empty email falls
/// back to the most recently used account for compatibility with old callers.
fn cred_for(cfg: &Config, email: &str) -> Option<crate::models::Credentials> {
    let accounts = crate::credentials::load_all(&cfg.encryption_password, &cfg.account_format);
    accounts
        .into_iter()
        .find(|c| {
            !c.decrypt_failed
                && (email.is_empty() || c.creds.email.eq_ignore_ascii_case(email))
        })
        .map(|c| c.creds)
}

fn options_for(cfg: &Config, browser: PathBuf) -> RegisterOptions {
    RegisterOptions {
        cfg: cfg.clone(),
        provider: cfg.email_provider.clone(),
        proxy: if cfg.proxy.is_empty() {
            None
        } else {
            Some(cfg.proxy.clone())
        },
        visible: cfg.visible_browser,
        browser,
        keep_profile: false,
    }
}

async fn create_batch(cfg: &Config, total: u64, workers: usize, tx: &Sender<UiEvent>) {
    let started = Instant::now();
    let workers = workers.clamp(1, total.max(1) as usize);
    console::event(Event::RunStarted {
        kind: "create".to_string(),
        total,
        workers,
    });
    let Some(browser) = resolve_browser(cfg, None) else {
        let _ = tx.send(UiEvent::Toast {
            kind: "error".to_string(),
            text: "No Chrome, Edge, Brave or Chromium install was found.".to_string(),
        });
        return;
    };

    let next = Arc::new(AtomicU64::new(0));
    let ok = Arc::new(AtomicU64::new(0));
    let failed = Arc::new(AtomicU64::new(0));
    let mut tasks = tokio::task::JoinSet::new();

    for worker in 0..workers {
        let options = options_for(cfg, browser.clone());
        let next = next.clone();
        let ok = ok.clone();
        let failed = failed.clone();
        let tx = tx.clone();
        tasks.spawn(console::labelled(format!("w{}", worker + 1), async move {
            loop {
                let index = next.fetch_add(1, Ordering::Relaxed);
                if index >= total {
                    break;
                }
                match console::collected(register_with_retries(&options)).await {
                    Ok(_) => {
                        ok.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(e) => {
                        failed.fetch_add(1, Ordering::Relaxed);
                        let _ = tx.send(UiEvent::Toast {
                            kind: "error".to_string(),
                            text: format!("{e:#}"),
                        });
                    }
                }
                let ok_now = ok.load(Ordering::Relaxed);
                let failed_now = failed.load(Ordering::Relaxed);
                console::event(Event::RunProgress {
                    done: ok_now + failed_now,
                    total,
                    ok: ok_now,
                    failed: failed_now,
                });
                let _ = tx.send(UiEvent::Progress {
                    done: ok_now + failed_now,
                    total,
                    ok: ok_now,
                    failed: failed_now,
                });
            }
        }));
    }
    while tasks.join_next().await.is_some() {}

    let ok = ok.load(Ordering::Relaxed);
    let failed = failed.load(Ordering::Relaxed);
    let seconds = started.elapsed().as_secs_f64();
    console::event(Event::RunFinished { ok, failed, seconds });
    let _ = tx.send(UiEvent::Finished {
        ok,
        failed,
        seconds,
    });
}

async fn keepalive(cfg: &Config, prune: bool, tx: &Sender<UiEvent>) {
    let workers = if cfg.visible_browser { 1 } else { 8 };
    match alive::check_all(cfg, workers, prune, prune, true).await {
        Ok((results, summary)) => {
            let _ = tx.send(UiEvent::KeepaliveResults(results, summary));
        }
        Err(e) => {
            let _ = tx.send(UiEvent::Toast {
                kind: "error".to_string(),
                text: format!("{e:#}"),
            });
        }
    }
}
