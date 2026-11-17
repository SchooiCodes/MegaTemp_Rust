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
use crate::services::alive::{self, AliveResult, KeepaliveSummary};

/// Work requested by the UI.
#[derive(Debug)]
pub enum Command {
    Create { total: u64, workers: usize },
    Keepalive { prune: bool },
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
                while let Ok(command) = command_rx.recv() {
                    match command {
                        Command::Create { total, workers } => {
                            create_batch(&cfg, total, workers, &event_tx).await;
                        }
                        Command::Keepalive { prune } => {
                            keepalive(&cfg, prune, &event_tx).await;
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
