//! GUI application shell: navigation rail, top bar and screens.

use eframe::egui::{self, RichText};
use egui_extras::{Column, TableBuilder};
use std::time::{Duration, Instant};

use crate::gui::bridge::{Bridge, CloudFile, Command, UiEvent};
use crate::browser::register::resolve_browser;
use crate::console::Color;
use crate::gui::theme as t;
use crate::models::Config;
use crate::{config, credentials, console};

#[derive(Copy, Clone, PartialEq, Eq)]
enum Screen {
    Dashboard,
    Create,
    Keepalive,
    Cloud,
    Accounts,
    Settings,
}

impl Screen {
    const ALL: [Screen; 6] = [
        Screen::Dashboard,
        Screen::Create,
        Screen::Keepalive,
        Screen::Cloud,
        Screen::Accounts,
        Screen::Settings,
    ];

    fn label(self) -> &'static str {
        match self {
            Screen::Dashboard => "Overview",
            Screen::Create => "Create",
            Screen::Keepalive => "Keep alive",
            Screen::Cloud => "Cloud",
            Screen::Accounts => "Accounts",
            Screen::Settings => "Settings",
        }
    }

    /// One-line explanation shown when hovering the navigation rail.
    fn description(self) -> &'static str {
        match self {
            Screen::Dashboard => "Overview — search saved accounts and open their details",
            Screen::Create => "Create — register new MEGA accounts in parallel",
            Screen::Keepalive => "Keep alive — log in to every account and check its storage",
            Screen::Cloud => "Cloud — browse, upload and download files",
            Screen::Accounts => "Accounts — edit tags and notes, copy passwords, manage accounts",
            Screen::Settings => "Settings — configure registration, exports and proxies",
        }
    }
}

/// A short-lived message shown on every screen.
struct Toast {
    text: String,
    kind: String,
    at: Instant,
}

/// One account as the dashboard table needs it.
#[derive(Clone)]
struct Row {
    email: String,
    tags: String,
    notes: String,
    password: String,
    age_days: i64,
    last_login_days: Option<i64>,
    locked: bool,
}

pub struct App {
    cfg: Config,
    screen: Screen,
    rows: Vec<Row>,
    filter: String,
    account_filter: String,
    bridge: Bridge,
    create_total: u64,
    create_workers: usize,
    create_provider: String,
    created: Vec<(String, String, f64)>,
    activity: Vec<(String, Color)>,
    status: Option<String>,
    progress: Option<(u64, u64, u64, u64)>,
    busy: bool,
    prune: bool,
    keep_rows: Vec<(String, bool, Option<f64>, Option<String>)>,
    keep_summary: Option<(usize, usize, usize, usize)>,
    keep_filter: String,
    toasts: Vec<Toast>,
    help_open: bool,
    cloud_loaded: bool,
    cloud_files: Vec<CloudFile>,
    cloud_filter: String,
    cloud_sort: u8,
    selected_row: usize,
    selected_email: String,
    edit_tags: String,
    edit_notes: String,
    settings: Config,
    reveal_password: bool,
    confirm_delete: bool,
    folder_name: String,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        t::apply(&cc.egui_ctx);
        let cfg = config::load(None);
        let mut app = Self {
            create_total: 1,
            create_workers: 1,
            create_provider: cfg.email_provider.clone(),
            bridge: Bridge::start(cfg.clone()),
            cfg: cfg.clone(),
            screen: Screen::Dashboard,
            rows: Vec::new(),
            filter: String::new(),
            account_filter: String::new(),
            created: Vec::new(),
            activity: Vec::new(),
            status: None,
            progress: None,
            busy: false,
            prune: false,
            keep_rows: Vec::new(),
            keep_summary: None,
            keep_filter: String::new(),
            toasts: Vec::new(),
            help_open: false,
            cloud_loaded: false,
            cloud_files: Vec::new(),
            cloud_filter: String::new(),
            cloud_sort: 0,
            selected_row: 0,
            selected_email: String::new(),
            edit_tags: String::new(),
            edit_notes: String::new(),
            settings: cfg.clone(),
            reveal_password: false,
            confirm_delete: false,
            folder_name: String::new(),
        };
        app.reload();
        app
    }

    /// Show a message on every screen, not just Settings.
    fn push_toast(&mut self, kind: &str, text: impl Into<String>) {
        self.toasts.push(Toast { text: text.into(), kind: kind.to_string(), at: Instant::now() });
        if self.toasts.len() > 4 {
            self.toasts.remove(0);
        }
    }

    fn matching_rows(&self, needle: &str) -> Vec<Row> {
        let needle = needle.trim().to_lowercase();
        self.rows
            .iter()
            .filter(|row| {
                needle.is_empty()
                    || row.email.to_lowercase().contains(&needle)
                    || row.tags.to_lowercase().contains(&needle)
            })
            .cloned()
            .collect()
    }

    /// Open an account from the dashboard in the Accounts screen.
    fn open_account(&mut self, email: String) {
        if let Some(index) = self.rows.iter().position(|row| row.email == email) {
            self.selected_row = index;
            self.selected_email.clear();
            self.account_filter.clear();
            self.screen = Screen::Accounts;
        }
    }

    /// Keep the detail form in step with the highlighted row, including after
    /// a reload or a filter change.
    fn sync_selection(&mut self) {
        let rows = self.matching_rows(&self.account_filter.clone());
        if rows.is_empty() {
            self.selected_row = 0;
            return;
        }
        if self.selected_row >= rows.len() {
            self.selected_row = 0;
        }
        let row = &rows[self.selected_row];
        if row.email != self.selected_email {
            self.selected_email = row.email.clone();
            self.edit_tags = row.tags.clone();
            self.edit_notes = row.notes.clone();
            self.reveal_password = false;
            self.confirm_delete = false;
        }
    }

    /// Pull everything the worker runtime produced since the last frame.
    fn drain_events(&mut self) {
        while let Ok(event) = self.bridge.events.try_recv() {
            match event {
                UiEvent::Line { text, colour, .. } => {
                    self.activity.push((text, colour));
                    if self.activity.len() > 500 {
                        self.activity.remove(0);
                    }
                }
                UiEvent::Status(text) => self.status = Some(text),
                UiEvent::Progress {
                    done,
                    total,
                    ok,
                    failed,
                } => self.progress = Some((done, total, ok, failed)),
                UiEvent::Created {
                    email,
                    password,
                    seconds,
                } => self.created.insert(0, (email, password, seconds)),
                UiEvent::Finished { ok, failed, .. } => {
                    self.busy = false;
                    self.status = None;
                    self.push_toast("success", format!("Run finished: {ok} created, {failed} failed"));
                    self.reload();
                }
                UiEvent::KeepaliveRow {
                    email,
                    alive,
                    quota_gb,
                    error,
                } => {
                    self.keep_rows.retain(|row| row.0 != email);
                    self.keep_rows.push((email, alive, quota_gb, error));
                }
                UiEvent::KeepaliveResults(results, summary) => {
                    self.busy = false;
                    self.status = None;
                    self.keep_rows = results
                        .into_iter()
                        .map(|row| (row.email, row.status == "alive", row.quota_gb, row.error))
                        .collect();
                    self.keep_summary =
                        Some((summary.total, summary.alive, summary.dead, summary.pruned));
                    self.push_toast(
                        "success",
                        format!("Keepalive: {} alive, {} dead", summary.alive, summary.dead),
                    );
                }
                UiEvent::CloudFiles(files) => {
                    self.cloud_files = files;
                    self.cloud_loaded = true;
                    self.busy = false;
                    self.status = None;
                }
                UiEvent::Toast { kind, text } => {
                    self.push_toast(&kind, if kind == "error" { format!("Error: {text}") } else { text });
                }
                UiEvent::Idle => {
                    self.busy = false;
                    self.status = None;
                }
            }
        }
    }

    fn start_create(&mut self) {
        self.created.clear();
        self.activity.clear();
        self.toasts.clear();
        let mut run_cfg = self.cfg.clone();
        run_cfg.email_provider = self.create_provider.trim().to_string();
        if run_cfg.email_provider.is_empty() {
            self.push_toast("error", "Choose an email provider first");
            return;
        }
        if resolve_browser(&run_cfg, None).is_none() {
            self.push_toast(
                "error",
                "No browser found. Install Chromium in Settings or set a browser path.",
            );
            return;
        }
        self.busy = true;
        self.progress = Some((0, self.create_total, 0, 0));
        let _ = self.bridge.commands.send(Command::Create {
            total: self.create_total,
            workers: self.create_workers,
            cfg: run_cfg,
        });
    }

    fn start_keepalive(&mut self) {
        self.status = None;
        self.keep_rows.clear();
        self.keep_summary = None;
        self.activity.clear();
        self.toasts.clear();
        self.busy = true;
        let _ = self
            .bridge
            .commands
            .send(Command::Keepalive { prune: self.prune });
    }

    fn reload(&mut self) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        self.rows = credentials::load_all(&self.cfg.encryption_password, &self.cfg.account_format)
            .into_iter()
            .map(|account| {
                let age_days = if account.mtime > 0.0 {
                    ((now - account.mtime) / 86_400.0).max(0.0) as i64
                } else {
                    0
                };
                let last_login_days = (account.creds.last_login > 0.0)
                    .then(|| ((now - account.creds.last_login) / 86_400.0).max(0.0) as i64);
                Row {
                    email: account.creds.email,
                    tags: account.creds.tags,
                    notes: account.creds.notes,
                    password: account.creds.password,
                    age_days,
                    last_login_days,
                    locked: account.decrypt_failed,
                }
            })
            .collect();
    }

    fn visible_rows(&self) -> Vec<Row> {
        self.matching_rows(&self.filter)
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("top")
            .exact_size(58.0)
            .frame(
                egui::Frame::new()
                    .fill(t::SURFACE)
                    .inner_margin(egui::Margin::symmetric(20, 0)),
            )
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    let mark = egui::Frame::new()
                        .fill(t::ACCENT)
                        .corner_radius(egui::CornerRadius::same(7))
                        .inner_margin(egui::Margin::symmetric(8, 5));
                    mark.show(ui, |ui| {
                        ui.label(RichText::new("M").size(15.0).color(egui::Color32::WHITE).strong());
                    });
                    ui.add_space(4.0);
                    ui.label(RichText::new("MegaTemp").size(16.0).color(t::TEXT).strong());
                    ui.label(RichText::new(console::VERSION).size(12.0).color(t::FAINT));

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .small_button("?")
                            .on_hover_text("Quick guide and keyboard shortcuts")
                            .clicked()
                        {
                            self.help_open = true;
                        }
                        ui.add_space(10.0);
                        ui.label(RichText::new(self.screen.label()).size(12.0).color(t::MUTED));
                        ui.add_space(10.0);
                        let (label, colour) = if self.busy {
                            ("Working", t::ACCENT)
                        } else {
                            ("Idle", t::MUTED)
                        };
                        // Painted rather than a glyph: the default font has no
                        // reliable round dot, and a status pill should never
                        // depend on emoji fallbacks.
                        let (rect, _) =
                            ui.allocate_exact_size(egui::Vec2::new(74.0, 26.0), egui::Sense::hover());
                        let painter = ui.painter();
                        painter.rect(
                            rect,
                            egui::CornerRadius::same(255),
                            t::RAISED,
                            egui::Stroke::new(1.0, t::BORDER),
                            egui::StrokeKind::Inside,
                        );
                        painter.circle_filled(
                            egui::pos2(rect.left() + 16.0, rect.center().y),
                            3.5,
                            colour,
                        );
                        painter.text(
                            egui::pos2(rect.left() + 27.0, rect.center().y),
                            egui::Align2::LEFT_CENTER,
                            label,
                            egui::FontId::proportional(12.0),
                            colour,
                        );
                    });
                });
            });
    }

    fn rail(&mut self, ui: &mut egui::Ui) {
        egui::Panel::left("rail")
            .exact_size(104.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(t::SURFACE)
                    .inner_margin(egui::Margin::symmetric(8, 16)),
            )
            .show(ui, |ui| {
                // Centre the whole nav group vertically in the rail instead of
                // pinning it to the top edge.
                let count = Screen::ALL.len() as f32;
                let content_height = count * 58.0 + (count - 1.0) * 4.0;
                let top = ((ui.available_height() - content_height) / 2.0).max(0.0);
                ui.add_space(top);
                ui.vertical_centered(|ui| {
                    for screen in Screen::ALL {
                        let response = nav_button(ui, screen, self.screen == screen);
                        let response = response.on_hover_text(screen.description());
                        if response.clicked() {
                            self.screen = screen;
                        }
                        ui.add_space(4.0);
                    }
                });
            });
    }

    fn dashboard(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new("Accounts").size(24.0).strong().color(t::TEXT));
                ui.label(
                    RichText::new(format!(
                        "{} saved · {} shown",
                        self.rows.len(),
                        self.visible_rows().len()
                    ))
                    .size(12.0)
                    .color(t::MUTED),
                );
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add(
                        egui::Button::new(RichText::new("New account").color(egui::Color32::WHITE))
                            .fill(t::ACCENT)
                            .min_size(egui::Vec2::new(120.0, 34.0)),
                    )
                    .clicked()
                {
                    self.screen = Screen::Create;
                }
                if ui.button("Refresh").clicked() {
                    self.reload();
                }
                if !self.filter.is_empty() && ui.button("Clear").clicked() {
                    self.filter.clear();
                }
                ui.add(
                    egui::TextEdit::singleline(&mut self.filter)
                        .hint_text("Search email or tags")
                        .desired_width(220.0),
                );
            });
        });

        ui.add_space(18.0);
        let total = self.rows.len();
        let locked = self.rows.iter().filter(|r| r.locked).count();
        let with_tags = self.rows.iter().filter(|r| !r.tags.is_empty()).count();
        ui.columns(3, |cols| {
            stat_card(&mut cols[0], "Saved accounts", &total.to_string(), t::TEXT);
            stat_card(&mut cols[1], "Tagged", &with_tags.to_string(), t::TEXT);
            stat_card(
                &mut cols[2],
                "Locked files",
                &locked.to_string(),
                if locked > 0 { t::WARN } else { t::MUTED },
            );
        });

        ui.add_space(18.0);
        let visible = self.visible_rows();
        let mut open_email = None;
        t::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            if visible.is_empty() {
                ui.add_space(24.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("No accounts yet").size(16.0).color(t::TEXT).strong());
                    ui.label(
                        RichText::new("Create your first MEGA account to see it here.")
                            .size(13.0)
                            .color(t::MUTED),
                    );
                    ui.add_space(8.0);
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new("Create accounts").color(egui::Color32::WHITE),
                            )
                            .fill(t::ACCENT),
                        )
                        .clicked()
                    {
                        self.screen = Screen::Create;
                    }
                });
                ui.add_space(24.0);
                return;
            }

            ui.label(
                RichText::new(format!("Showing {} of {} accounts", visible.len(), total))
                    .size(11.0)
                    .color(t::FAINT),
            );
            let table_height = (ui.available_height() - 86.0).max(180.0);
            TableBuilder::new(ui)
                .striped(true)
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                .max_scroll_height(table_height)
                .column(Column::initial(290.0).at_least(220.0))
                .column(Column::remainder().at_least(90.0))
                .column(Column::initial(180.0).at_least(120.0))
                .column(Column::exact(70.0))
                .column(Column::exact(100.0))
                .header(30.0, |mut header| {
                    for title in ["Email", "Tags", "Password", "Age", "Last login"] {
                        header.col(|ui| {
                            ui.label(t::section_label(title));
                        });
                    }
                })
                .body(|body| {
                    body.rows(34.0, visible.len(), |mut row| {
                        let account = &visible[row.index()];
                        row.col(|ui| {
                            let button = egui::Button::new(
                                RichText::new(&account.email).monospace().size(12.0).color(t::TEXT),
                            )
                            .frame(false);
                            if ui.add(button).on_hover_text("Open account details").clicked() {
                                open_email = Some(account.email.clone());
                            }
                        });
                        row.col(|ui| {
                            if account.tags.is_empty() {
                                ui.label(RichText::new("—").color(t::FAINT));
                            } else {
                                ui.label(RichText::new(&account.tags).size(12.0).color(t::MUTED));
                            }
                        });
                        row.col(|ui| {
                            let masked = mask(&account.password);
                            ui.label(RichText::new(masked).monospace().color(t::MUTED));
                        });
                        row.col(|ui| {
                            ui.label(
                                RichText::new(format!("{}d", account.age_days))
                                    .size(12.0)
                                    .color(t::MUTED),
                            );
                        });
                        row.col(|ui| {
                            let text = match account.last_login_days {
                                Some(days) => format!("{days}d ago"),
                                None => "never".to_string(),
                            };
                            ui.label(RichText::new(text).size(12.0).color(t::MUTED));
                        });
                    });
                });
        });
        if let Some(email) = open_email {
            self.open_account(email);
        }
    }

    fn create_screen(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(
                    RichText::new("Create accounts")
                        .size(24.0)
                        .strong()
                        .color(t::TEXT),
                );
                ui.add_space(4.0);
                ui.label(
                    RichText::new(
                        "Register MEGA accounts with disposable mailboxes. Workers run in parallel; attempts retry a failed signup.",
                    )
                    .size(13.0)
                    .color(t::MUTED),
                );
            });
            ui.with_layout(
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    let label = if self.busy { "Working…" } else { "Start run" };
                    let button = egui::Button::new(
                        RichText::new(label)
                            .size(14.0)
                            .strong()
                            .color(egui::Color32::WHITE),
                    )
                    .fill(if self.busy { t::RAISED } else { t::ACCENT })
                    .corner_radius(egui::CornerRadius::same(9))
                    .min_size(egui::vec2(132.0, 36.0));
                    if ui.add_enabled(!self.busy, button).clicked() {
                        self.start_create();
                    }
                },
            );
        });
        ui.add_space(18.0);

        t::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(t::section_label("Batch"));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Accounts").size(13.0).color(t::MUTED))
                    .on_hover_text("How many accounts to register in this run.");
                ui.add(
                    egui::DragValue::new(&mut self.create_total)
                        .range(1..=500)
                        .speed(1.0),
                );
                ui.add_space(18.0);
                ui.label(RichText::new("Workers").size(13.0).color(t::MUTED))
                    .on_hover_text("Accounts created at the same time; each worker has its own browser.");
                ui.add(
                    egui::DragValue::new(&mut self.create_workers)
                        .range(1..=32)
                        .speed(1.0),
                );
                ui.add_space(18.0);
                ui.label(RichText::new("Provider").size(13.0).color(t::MUTED))
                    .on_hover_text("Disposable inbox service used for MEGA's confirmation email.");
                ui.add(
                    egui::TextEdit::singleline(&mut self.create_provider)
                        .desired_width(150.0)
                        .hint_text("mailtm"),
                );
                ui.add_space(18.0);
                ui.label(RichText::new("Attempts").size(13.0).color(t::MUTED))
                    .on_hover_text("Tries per account if a signup fails.");
                ui.add(egui::DragValue::new(&mut self.cfg.max_attempts).range(1..=20));
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                for provider in ["mailtm", "guerrillamail", "mailgw"] {
                    let selected = self.create_provider.eq_ignore_ascii_case(provider);
                    let colour = if selected { t::TEXT } else { t::MUTED };
                    let btn = egui::Button::new(
                        RichText::new(provider).size(12.0).color(colour).strong(),
                    )
                    .fill(if selected {
                        colour.gamma_multiply(0.25)
                    } else {
                        t::RAISED
                    })
                    .stroke(egui::Stroke::new(1.0, colour.gamma_multiply(0.55)))
                    .corner_radius(egui::CornerRadius::same(255))
                    .min_size(egui::vec2(60.0, 24.0));
                    if ui.add(btn).clicked() {
                        self.create_provider = provider.to_string();
                    }
                }
                ui.add_space(10.0);
                ui.checkbox(&mut self.cfg.visible_browser, "Show the browser");
            });
            ui.add_space(6.0);
            ui.label(
                RichText::new("Workers run in parallel. Failed signups retry automatically; tick Show the browser to watch the registration happen.")
                    .size(11.0)
                    .color(t::FAINT),
            );
        });

        if self.progress.is_none() && !self.busy {
            ui.add_space(14.0);
            t::card().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.vertical_centered(|ui| {
                    ui.add_space(6.0);
                    ui.label(RichText::new("Ready when you are").size(14.0).strong().color(t::TEXT));
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new("Choose the batch size and press Start run. Each account gets its own disposable inbox and browser.")
                            .size(12.0)
                            .color(t::MUTED),
                    );
                    ui.add_space(6.0);
                });
            });
        }

        if let Some((done, total, ok, failed)) = self.progress {
            ui.add_space(14.0);
            t::card().show(ui, |ui| {
                ui.set_width(ui.available_width());
                let ratio = if total == 0 {
                    0.0
                } else {
                    done as f32 / total as f32
                };
                ui.add(
                    egui::ProgressBar::new(ratio)
                        .desired_height(8.0)
                        .fill(t::ACCENT)
                        .text(format!("{done}/{total} · {ok} ok · {failed} failed")),
                );
                if let Some(status) = &self.status {
                    ui.add_space(6.0);
                    ui.label(RichText::new(status).size(12.0).color(t::MUTED));
                }
            });
        }

        if !self.created.is_empty() {
            ui.add_space(14.0);
            t::card().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(t::section_label(&format!("Created this run ({})", self.created.len())));
                ui.add_space(6.0);
                egui::ScrollArea::vertical()
                    .max_height(220.0)
                    .show(ui, |ui| {
                        for (email, password, seconds) in &self.created {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(email).monospace().size(12.0).color(t::TEXT));
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.label(
                                            RichText::new(format!("{seconds:.1}s"))
                                                .size(11.0)
                                                .color(t::MUTED),
                                        );
                                        ui.label(
                                            RichText::new(password)
                                                .monospace()
                                                .size(12.0)
                                                .color(t::OK),
                                        );
                                        ui.label(
                                            RichText::new("password").size(10.0).color(t::FAINT),
                                        );
                                    },
                                );
                            });
                        }
                    });
            });
        }

        if !self.activity.is_empty() {
            ui.add_space(14.0);
            t::card().show(ui, |ui| {
                ui.set_width(ui.available_width());
                egui::CollapsingHeader::new(
                    RichText::new(format!("Activity ({})", self.activity.len()))
                        .size(12.0)
                        .color(t::MUTED),
                )
                .default_open(self.busy)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                        for (line, colour) in &self.activity {
                            ui.label(
                                RichText::new(line)
                                    .monospace()
                                    .size(11.5)
                                    .color(ui_colour(*colour)),
                            );
                        }
                    });
                });
            });
        }
    }

    fn keepalive_screen(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(
                    RichText::new("Keep alive")
                        .size(24.0)
                        .strong()
                        .color(t::TEXT),
                );
                ui.add_space(4.0);
                ui.label(
                    RichText::new(
                        "Log every saved account in, read its free storage and prune the dead ones.",
                    )
                    .size(13.0)
                    .color(t::MUTED),
                );
            });
            ui.with_layout(
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    let label = if self.busy { "Working…" } else { "Check all" };
                    let button = egui::Button::new(
                        RichText::new(label)
                            .size(14.0)
                            .strong()
                            .color(egui::Color32::WHITE),
                    )
                    .fill(if self.busy { t::RAISED } else { t::ACCENT })
                    .corner_radius(egui::CornerRadius::same(9))
                    .min_size(egui::vec2(132.0, 36.0));
                    if ui.add_enabled(!self.busy, button).clicked() {
                        self.start_keepalive();
                    }
                },
            );
        });
        ui.add_space(18.0);

        t::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.prune, "Remove dead accounts")
                    .on_hover_text("Dead accounts are moved to credentials/pruned/ before deletion.");
                ui.add_space(12.0);
                if let Some(status) = &self.status {
                    ui.label(RichText::new(status).size(12.0).color(t::MUTED));
                }
            });
            ui.add_space(4.0);
            ui.label(
                RichText::new("Every saved account is logged in and its free storage is read. This does not create or delete files.")
                    .size(11.0)
                    .color(t::FAINT),
            );
        });

        if let Some((total, alive, dead, pruned)) = self.keep_summary {
            ui.add_space(14.0);
            ui.columns(4, |cols| {
                for (index, (label, value, colour)) in [
                    ("Total", total, t::TEXT),
                    ("Alive", alive, t::OK),
                    ("Dead", dead, if dead == 0 { t::OK } else { t::WARN }),
                    ("Pruned", pruned, t::MUTED),
                ]
                .into_iter()
                .enumerate()
                {
                    t::card().show(&mut cols[index], |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(t::section_label(label));
                        ui.label(
                            RichText::new(value.to_string())
                                .size(26.0)
                                .strong()
                                .color(colour),
                        );
                    });
                }
            });
        }

        ui.add_space(14.0);
        t::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.keep_filter)
                        .hint_text("Find email or error")
                        .desired_width(260.0),
                );
                if !self.keep_filter.is_empty() && ui.button("Clear").clicked() {
                    self.keep_filter.clear();
                }
                if let Some(status) = &self.status {
                    ui.label(RichText::new(status).size(12.0).color(t::MUTED));
                }
            });
            ui.add_space(8.0);

            let needle = self.keep_filter.trim().to_lowercase();
            let rows: Vec<_> = self
                .keep_rows
                .iter()
                .filter(|(email, _, _, error)| {
                    needle.is_empty()
                        || email.to_lowercase().contains(&needle)
                        || error.as_deref().unwrap_or("").to_lowercase().contains(&needle)
                })
                .cloned()
                .collect();

            if self.keep_rows.is_empty() {
                ui.add_space(10.0);
                ui.label(
                    RichText::new("No results yet. Press Check all to log in to every saved account.")
                        .size(13.0)
                        .color(t::MUTED),
                );
                ui.add_space(10.0);
                return;
            }
            if rows.is_empty() {
                ui.add_space(10.0);
                ui.label(
                    RichText::new("No accounts match this search.")
                        .size(13.0)
                        .color(t::MUTED),
                );
                ui.add_space(10.0);
                return;
            }
            ui.label(
                RichText::new(format!(
                    "Showing {} of {} accounts",
                    rows.len(),
                    self.keep_rows.len()
                ))
                .size(11.0)
                .color(t::FAINT),
            );
            let table_height = (ui.available_height() - 66.0).max(180.0);
            TableBuilder::new(ui)
                .striped(true)
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                .max_scroll_height(table_height)
                .column(Column::initial(300.0).at_least(220.0))
                .column(Column::exact(90.0))
                .column(Column::exact(100.0))
                .column(Column::remainder().at_least(160.0))
                .header(30.0, |mut header| {
                    for title in ["Email", "Status", "Storage", "Detail"] {
                        header.col(|ui| {
                            ui.label(t::section_label(title));
                        });
                    }
                })
                .body(|body| {
                    body.rows(34.0, rows.len(), |mut row| {
                        let (email, alive, quota, error) = &rows[row.index()];
                        row.col(|ui| {
                            ui.label(RichText::new(email).monospace().size(12.0).color(t::TEXT));
                        });
                        row.col(|ui| {
                            let (text, colour) = if *alive {
                                ("alive", t::OK)
                            } else {
                                ("dead", t::DANGER)
                            };
                            t::chip(ui, text, colour);
                        });
                        row.col(|ui| {
                            let text = quota
                                .map(|q| format!("{q:.1} GB"))
                                .unwrap_or_else(|| "—".to_string());
                            ui.label(RichText::new(text).size(12.0).color(t::MUTED));
                        });
                        row.col(|ui| {
                            if let Some(error) = error {
                                ui.label(
                                    RichText::new(error).size(11.0).color(t::DANGER),
                                );
                            }
                        });
                    });
                });
        });
    }

    fn cloud_screen(&mut self, ui: &mut egui::Ui) {
        // Load once when the screen is first opened. The bridge emits
        // CloudFiles, which fills the list below.
        if !self.cloud_loaded && !self.busy {
            self.cloud_loaded = true;
            self.busy = true;
            self.status = Some("Loading cloud files...".to_string());
            let _ = self.bridge.commands.send(Command::ListCloud);
        }
        ui.label(RichText::new("Cloud").size(24.0).strong().color(t::TEXT));
        ui.add_space(4.0);
        let account = self
            .rows
            .first()
            .map(|row| row.email.clone())
            .unwrap_or_else(|| "no saved account".to_string());
        ui.label(
            RichText::new(format!("Files in {account}"))
                .size(13.0)
                .color(t::MUTED),
        );
        ui.add_space(18.0);

        let files = self.cloud_files.clone();
        let total: u64 = files.iter().map(|file| file.size).sum();
        let mut refresh_clicked = false;
        let mut upload_clicked = false;
        t::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new(format!(
                            "{} file{}",
                            files.len(),
                            if files.len() == 1 { "" } else { "s" }
                        ))
                        .size(17.0)
                        .strong()
                        .color(t::TEXT),
                    );
                    ui.label(
                        RichText::new(format!("{} total", crate::util::human_size(total)))
                            .size(11.5)
                            .color(t::MUTED),
                    );
                });
                // Push the actions to the right edge without over-allocating:
                // `available_width()` here is the space left after the summary,
                // so leave just enough for the two buttons.
                let gap = (ui.available_width() - 200.0).max(8.0);
                ui.add_space(gap);
                if ui.button("Refresh").clicked() {
                    refresh_clicked = true;
                }
                if ui.button(RichText::new("Upload file").color(t::TEXT)).clicked() {
                    upload_clicked = true;
                }
            });
            if let Some(status) = &self.status {
                ui.add_space(6.0);
                ui.label(RichText::new(status).size(12.0).color(t::MUTED));
            }
        });
        if refresh_clicked && !self.busy {
            self.busy = true;
            self.status = Some("Loading cloud files...".to_string());
            let _ = self.bridge.commands.send(Command::ListCloud);
        }
        if upload_clicked && !self.busy {
            if let Some(path) = rfd::FileDialog::new().pick_file() {
                self.busy = true;
                self.status = Some(format!(
                    "Uploading {}...",
                    path.file_name().unwrap_or_default().to_string_lossy()
                ));
                let _ = self.bridge.commands.send(Command::UploadFile { path });
            }
        }
        ui.add_space(12.0);

        t::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new("Find").color(t::MUTED));
                ui.add(
                    egui::TextEdit::singleline(&mut self.cloud_filter)
                        .hint_text("name or handle")
                        .desired_width(240.0),
                );
                let sort_label = match self.cloud_sort {
                    0 => "Sort: Name",
                    1 => "Sort: Size",
                    _ => "Sort: Handle",
                };
                if ui.button(sort_label).clicked() {
                    self.cloud_sort = (self.cloud_sort + 1) % 3;
                }
                if !self.cloud_filter.is_empty() && ui.button("Clear").clicked() {
                    self.cloud_filter.clear();
                }
            });
        });
        ui.add_space(12.0);

        let needle = self.cloud_filter.trim().to_lowercase();
        let mut shown: Vec<CloudFile> = files
            .clone()
            .into_iter()
            .filter(|file| {
                needle.is_empty()
                    || file.name.to_lowercase().contains(&needle)
                    || file.handle.to_lowercase().contains(&needle)
            })
            .collect();
        match self.cloud_sort {
            0 => shown.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase())),
            1 => shown.sort_by(|a, b| b.size.cmp(&a.size)),
            _ => shown.sort_by(|a, b| a.handle.cmp(&b.handle)),
        }
        let shown_count = shown.len();
        let mut download_handle: Option<String> = None;
        let mut copy_handle: Option<String> = None;
        t::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            if files.is_empty() {
                ui.add_space(14.0);
                ui.vertical_centered(|ui| {
                    ui.label(
                        RichText::new("No files on this account yet")
                            .size(14.0)
                            .strong()
                            .color(t::TEXT),
                    );
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new("Use Upload file above to put something in the cloud.")
                            .size(12.0)
                            .color(t::MUTED),
                    );
                });
                ui.add_space(14.0);
                return;
            }
            if shown.is_empty() {
                ui.add_space(14.0);
                ui.label(
                    RichText::new("No files match the search.")
                        .size(13.0)
                        .color(t::MUTED),
                );
                ui.add_space(14.0);
                return;
            }
            let table_height = (ui.available_height() - 40.0).max(180.0);
            TableBuilder::new(ui)
                .striped(true)
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                .max_scroll_height(table_height)
                .column(Column::initial(300.0).at_least(200.0))
                .column(Column::exact(150.0))
                .column(Column::exact(100.0))
                .column(Column::remainder().at_least(190.0))
                .header(30.0, |mut header| {
                    for title in ["Name", "Handle", "Size", "Actions"] {
                        header.col(|ui| {
                            ui.label(t::section_label(title));
                        });
                    }
                })
                .body(|mut body| {
                    for file in &shown {
                        body.row(38.0, |mut row| {
                            row.col(|ui| {
                                ui.horizontal(|ui| {
                                    let (dot, _) = ui.allocate_exact_size(
                                        egui::vec2(8.0, 8.0),
                                        egui::Sense::hover(),
                                    );
                                    ui.painter().circle_filled(dot.center(), 4.0, t::ACCENT);
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(&file.name).color(t::TEXT),
                                        )
                                        .truncate(),
                                    )
                                    .on_hover_text(&file.name);
                                });
                            });
                            row.col(|ui| {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(&file.handle)
                                            .monospace()
                                            .size(11.0)
                                            .color(t::MUTED),
                                    )
                                    .truncate(),
                                )
                                .on_hover_text(&file.handle);
                            });
                            row.col(|ui| {
                                ui.label(
                                    RichText::new(crate::util::human_size(file.size))
                                        .size(12.0)
                                        .color(t::MUTED),
                                );
                            });
                            row.col(|ui| {
                                ui.horizontal(|ui| {
                                    if ui.button("Download").clicked() {
                                        download_handle = Some(file.handle.clone());
                                    }
                                    if ui.button("Copy").clicked() {
                                        copy_handle = Some(file.handle.clone());
                                    }
                                });
                            });
                        });
                    }
                });
            if shown_count != files.len() {
                ui.add_space(4.0);
                ui.label(
                    RichText::new(format!("Showing {shown_count} of {} files", files.len()))
                        .size(11.0)
                        .color(t::MUTED),
                );
            }
        });
        if let Some(handle) = copy_handle {
            ui.ctx().copy_text(handle.clone());
            self.push_toast("ok", format!("Copied handle {handle}"));
        }
        if let Some(handle) = download_handle {
            if let Some(dest) = rfd::FileDialog::new().pick_folder() {
                self.busy = true;
                self.status = Some(format!("Downloading {handle}..."));
                let _ = self
                    .bridge
                    .commands
                    .send(Command::DownloadFile { handle, dest });
            }
        }
    }

    fn accounts_screen(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Accounts").size(24.0).strong().color(t::TEXT));
        ui.add_space(4.0);
        ui.label(
            RichText::new("Inspect, edit and manage saved credentials.")
                .size(13.0)
                .color(t::MUTED),
        );
        ui.add_space(18.0);
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.account_filter)
                    .hint_text("Find email or tags"),
            );
            if ui.button("Reload").clicked() {
                self.reload();
            }
        });
        let rows = self.matching_rows(&self.account_filter.clone());
        if rows.is_empty() {
            ui.label("No accounts saved yet.");
            return;
        }
        if self.selected_row >= rows.len() {
            self.selected_row = 0;
        }
        ui.columns(2, |cols| {
            t::card().show(&mut cols[0], |ui| {
                ui.set_width(ui.available_width());
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for (i, row) in rows.iter().enumerate() {
                        let selected = i == self.selected_row;
                        if ui.selectable_label(selected, &row.email).clicked() {
                            self.selected_row = i;
                            self.selected_email = row.email.clone();
                            self.edit_tags = row.tags.clone();
                            self.edit_notes = row.notes.clone();
                            self.reveal_password = false;
                            self.confirm_delete = false;
                        }
                    }
                });
            });
            let row = rows[self.selected_row].clone();
            t::card().show(&mut cols[1], |ui| {
                ui.set_width(ui.available_width());
                ui.label(egui::RichText::new(&row.email).strong());
                ui.label(format!("Age: {}d", row.age_days));
                if let Some(days) = row.last_login_days {
                    ui.label(format!("Last login: {days}d ago"));
                } else {
                    ui.label("Last login: never");
                }
                if row.locked {
                    ui.colored_label(egui::Color32::from_rgb(255, 91, 96), "Locked (could not decrypt)");
                }
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label("Password:");
                    if self.reveal_password {
                        ui.monospace(&row.password);
                    } else {
                        ui.monospace(crate::util::mask(&row.password));
                    }
                    if ui.small_button(if self.reveal_password { "Hide" } else { "Reveal" }).clicked() {
                        self.reveal_password = !self.reveal_password;
                    }
                    if ui.small_button("Copy").clicked() {
                        let _ = crate::util::copy_to_clipboard(&row.password);
                        self.push_toast("ok", "Password copied");
                    }
                    if ui.small_button("Copy email").clicked() {
                        let _ = crate::util::copy_to_clipboard(&row.email);
                        self.push_toast("ok", "Email copied");
                    }
                });
                ui.separator();
                ui.label("Tags:");
                ui.text_edit_singleline(&mut self.edit_tags);
                ui.label("Notes:");
                ui.text_edit_multiline(&mut self.edit_notes);
                if ui.button("Save tags & notes").clicked() {
                    // persist via credentials update
                    if let Some(account) = crate::credentials::load_all(&self.cfg.encryption_password, &self.cfg.account_format)
                        .into_iter().find(|a| a.creds.email == row.email) {
                        let mut creds = account.creds;
                        creds.tags = self.edit_tags.clone();
                        creds.notes = self.edit_notes.clone();
                        if crate::credentials::update_credential(&creds.email, &creds, &self.cfg).is_ok() {
                            self.push_toast("ok", "Tags and notes saved");
                        } else {
                            self.push_toast("error", "Could not save this account");
                        }
                        self.reload();
                    }
                }
                ui.separator();
                ui.horizontal(|ui| {
                    ui.text_edit_singleline(&mut self.folder_name);
                    if ui.button("Create folder").clicked() && !self.folder_name.trim().is_empty() {
                        let name = self.folder_name.clone();
                        let _ = self.bridge.commands.send(Command::CreateFolder {
                            email: row.email.clone(),
                            name,
                        });
                        self.folder_name.clear();
                    }
                });
                ui.separator();
                if !self.confirm_delete {
                    if ui.button(egui::RichText::new("Delete account").color(t::DANGER)).clicked() {
                        self.confirm_delete = true;
                    }
                } else {
                    ui.label(
                        RichText::new(format!("Delete {}? This removes the saved credentials and account data.", row.email))
                            .size(12.0)
                            .color(t::DANGER),
                    );
                    ui.horizontal(|ui| {
                        if ui.button("Yes, delete").clicked() {
                            let _ = self.bridge.commands.send(Command::DeleteAccount { email: row.email.clone() });
                            self.confirm_delete = false;
                            self.selected_row = 0;
                        }
                        if ui.button("Cancel").clicked() {
                            self.confirm_delete = false;
                        }
                    });
                }
            });
        });
    }

    fn settings_screen(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Settings").size(24.0).strong().color(t::TEXT));
        ui.add_space(4.0);
        ui.label(
            RichText::new("Changes are saved to config.json and shared with the CLI.")
                .size(13.0)
                .color(t::MUTED),
        );
        ui.add_space(18.0);
        egui::ScrollArea::vertical().show(ui, |ui| {
            t::card().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(t::section_label("Registration"));
                egui::Grid::new("settings_registration").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                    ui.label("Email provider");
                    ui.text_edit_singleline(&mut self.settings.email_provider);
                    ui.end_row();
                    ui.label("Max attempts");
                    ui.add(egui::DragValue::new(&mut self.settings.max_attempts).range(1..=10));
                    ui.end_row();
                    ui.label("Mail timeout (s)");
                    ui.add(egui::DragValue::new(&mut self.settings.mail_timeout).range(30..=600));
                    ui.end_row();
                    ui.label("Visible browser");
                    ui.checkbox(&mut self.settings.visible_browser, "");
                    ui.end_row();
                    ui.label("Browser path");
                    ui.horizontal(|ui| {
                        ui.text_edit_singleline(&mut self.settings.executable_path);
                        if ui.button("Browse...").clicked() {
                            if let Some(path) = rfd::FileDialog::new().add_filter("Browser", &["exe"]).pick_file() {
                                self.settings.executable_path = path.to_string_lossy().to_string();
                            }
                        }
                    });
                    ui.end_row();
                });
            });
            ui.add_space(12.0);
            t::card().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(t::section_label("Exports"));
                egui::Grid::new("settings_exports").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                    ui.label("Auto CSV export");
                    ui.checkbox(&mut self.settings.csv_export, "");
                    ui.end_row();
                    ui.label("Auto JSONL export");
                    ui.checkbox(&mut self.settings.jsonl_export, "");
                    ui.end_row();
                    ui.label("Account format");
                    ui.text_edit_singleline(&mut self.settings.account_format);
                    ui.end_row();
                });
            });
            ui.add_space(12.0);
            t::card().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(t::section_label("Network & updates"));
                egui::Grid::new("settings_network").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                    ui.label("Proxy");
                    ui.text_edit_singleline(&mut self.settings.proxy);
                    ui.end_row();
                    ui.label("Proxy file");
                    ui.text_edit_singleline(&mut self.settings.proxy_file);
                    ui.end_row();
                    ui.label("Proxy per attempt");
                    ui.checkbox(&mut self.settings.proxy_per_attempt, "");
                    ui.end_row();
                    ui.label("Webhook URL");
                    ui.text_edit_singleline(&mut self.settings.webhook_url);
                    ui.end_row();
                    ui.label("Notifications / quiet");
                    ui.checkbox(&mut self.settings.quiet, "");
                    ui.end_row();
                });
            });
            ui.add_space(12.0);
            t::card().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(t::section_label("Security & credentials"));
                egui::Grid::new("settings_security").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                    ui.label("Encryption password");
                    ui.add(egui::TextEdit::singleline(&mut self.settings.encryption_password).password(true));
                    ui.end_row();
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let install = ui.add_enabled(!self.busy, egui::Button::new("Install Chromium"));
                    if install.clicked() {
                        self.busy = true;
                        let _ = self.bridge.commands.send(Command::InstallChromium);
                    }
                    if ui.button("Export credentials").clicked() {
                        match crate::services::extract::extract_credentials(&self.settings, &self.settings.account_format) {
                            Ok(path) => self.push_toast("ok", format!("Exported to {}", path.display())),
                            Err(e) => self.push_toast("error", format!("Export failed: {e}")),
                        }
                    }
                });
            });
            ui.add_space(16.0);
            ui.horizontal(|ui| {
                if ui.add(egui::Button::new("Save settings").fill(t::ACCENT)).clicked() {
                    match crate::config::save(None, &self.settings) {
                        Ok(()) => {
                            let _ = self.bridge.commands.send(Command::UpdateConfig(self.settings.clone()));
                            self.cfg = self.settings.clone();
                            self.reload();
                            self.push_toast("ok", "Settings saved");
                        }
                        Err(e) => self.push_toast("error", format!("Could not save settings: {e}")),
                    }
                }
                if ui.button("Reset").clicked() {
                    self.settings = self.cfg.clone();
                    self.push_toast("ok", "Changes discarded");
                }
            });
        });
    }

    fn placeholder(&mut self, ui: &mut egui::Ui, title: &str, body: &str) {
        ui.label(RichText::new(title).heading().strong());
        ui.add_space(6.0);
        ui.label(RichText::new(body).size(13.0).color(t::MUTED));
        ui.add_space(18.0);
        t::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add_space(12.0);
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new("This screen is being built")
                        .size(14.0)
                        .color(t::TEXT)
                        .strong(),
                );
                ui.label(
                    RichText::new("The core work is already wired; the UI lands in the next milestone.")
                        .size(12.0)
                        .color(t::MUTED),
                );
            });
            ui.add_space(12.0);
        });
    }

    /// Bottom-centred, auto-dismissing notifications for user actions.
    fn toast_overlay(&mut self, ui: &mut egui::Ui) {
        self.toasts
            .retain(|toast| toast.at.elapsed() < Duration::from_secs(6));
        let mut dismiss = None;
        egui::Area::new(egui::Id::unique("toast_overlay"))
            .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -18.0))
            .show(ui.ctx(), |ui| {
                if self.toasts.is_empty() {
                    return;
                }
                t::card().show(ui, |ui| {
                    ui.set_width(420.0);
                    for (index, toast) in self.toasts.iter().enumerate() {
                        let colour = match toast.kind.as_str() {
                            "ok" => t::OK,
                            "warn" => t::WARN,
                            "error" => t::DANGER,
                            _ => t::TEXT,
                        };
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(toast.text.clone()).size(12.5).color(colour));
                            if ui.button("×").on_hover_text("Dismiss").clicked() {
                                dismiss = Some(index);
                            }
                        });
                    }
                });
            });
        if let Some(index) = dismiss {
            self.toasts.remove(index);
        }
    }

    /// One-screen quick reference, opened with the `?` button or `?` shortcut.
    fn help_window(&mut self, ui: &mut egui::Ui) {
        egui::Window::new("Quick guide")
            .open(&mut self.help_open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ui.ctx(), |ui| {
                ui.set_width(420.0);
                ui.label(t::section_label("Screens"));
                for screen in Screen::ALL {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(screen.label()).strong().color(t::TEXT));
                        ui.label(RichText::new(screen.description()).size(12.0).color(t::MUTED));
                    });
                }
                ui.add_space(10.0);
                ui.label(t::section_label("Shortcuts"));
                for (key, text) in [
                    ("Ctrl + 1..6", "Jump to a screen"),
                    ("Ctrl + N", "Start the Create screen"),
                    ("Ctrl + R", "Refresh the current screen"),
                    ("Esc", "Back to Overview"),
                ] {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(key).monospace().color(t::ACCENT));
                        ui.label(RichText::new(text).size(12.0).color(t::MUTED));
                    });
                }
                ui.add_space(10.0);
                ui.label(
                    RichText::new("All CLI flags still work in the megatemp binary.")
                        .size(12.0)
                        .color(t::FAINT),
                );
            });
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.drain_events();
        if self.busy {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(120));
        }
        let mut goto = None;
        let mut refresh = false;
        ui.input(|input| {
            use egui::Key;
            if input.key_pressed(Key::Escape) {
                goto = Some(Screen::Dashboard);
            }
            if input.modifiers.ctrl {
                if input.key_pressed(Key::N) {
                    goto = Some(Screen::Create);
                }
                if input.key_pressed(Key::R) {
                    refresh = true;
                }
                for (screen, key) in Screen::ALL.iter().zip([
                    Key::Num1,
                    Key::Num2,
                    Key::Num3,
                    Key::Num4,
                    Key::Num5,
                    Key::Num6,
                ]) {
                    if input.key_pressed(key) {
                        goto = Some(*screen);
                    }
                }
            }
        });
        if let Some(screen) = goto {
            self.screen = screen;
        }
        if refresh {
            match self.screen {
                Screen::Dashboard => self.reload(),
                Screen::Cloud => {
                    if !self.busy {
                        self.busy = true;
                        let _ = self.bridge.commands.send(Command::ListCloud);
                    }
                }
                Screen::Keepalive => self.start_keepalive(),
                _ => {}
            }
        }
        self.top_bar(ui);
        self.rail(ui);
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(t::BG)
                    .inner_margin(egui::Margin::symmetric(28, 22)),
            )
            .show(ui, |ui| match self.screen {
                Screen::Dashboard => self.dashboard(ui),
                Screen::Create => self.create_screen(ui),
                Screen::Keepalive => self.keepalive_screen(ui),
                Screen::Cloud => self.cloud_screen(ui),
                Screen::Accounts => {
                    self.sync_selection();
                    self.accounts_screen(ui)
                }
                Screen::Settings => self.settings_screen(ui),
            });
        self.toast_overlay(ui);
        self.help_window(ui);
    }
}

/// One rail entry: a painted pictogram over the screen name.
///
/// Icons are drawn rather than set as text so the rail never depends on which
/// glyphs a platform font happens to carry.
fn nav_button(ui: &mut egui::Ui, screen: Screen, selected: bool) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(egui::Vec2::new(84.0, 58.0), egui::Sense::click());
    let hovered = response.hovered();
    let (fill, stroke) = if selected {
        (
            t::ACCENT.gamma_multiply(0.14),
            egui::Stroke::new(1.0, t::ACCENT.gamma_multiply(0.40)),
        )
    } else if hovered {
        (t::RAISED, egui::Stroke::NONE)
    } else {
        (egui::Color32::TRANSPARENT, egui::Stroke::NONE)
    };
    let painter = ui.painter();
    painter.rect(
        rect,
        egui::CornerRadius::same(10),
        fill,
        stroke,
        egui::StrokeKind::Inside,
    );
    let colour = if selected {
        t::ACCENT
    } else if hovered {
        t::TEXT
    } else {
        t::MUTED
    };
    draw_icon(
        painter,
        screen,
        egui::pos2(rect.center().x, rect.top() + 20.0),
        colour,
    );
    painter.text(
        egui::pos2(rect.center().x, rect.top() + 43.0),
        egui::Align2::CENTER_CENTER,
        screen.label(),
        egui::FontId::proportional(10.5),
        colour,
    );
    response
}

fn draw_icon(painter: &egui::Painter, screen: Screen, c: egui::Pos2, colour: egui::Color32) {
    let stroke = egui::Stroke::new(1.6, colour);
    let p = |x: f32, y: f32| egui::pos2(c.x + x, c.y + y);
    match screen {
        Screen::Dashboard => {
            // Roof, then the body below it.
            painter.line_segment([p(-7.0, -1.0), p(0.0, -7.0)], stroke);
            painter.line_segment([p(0.0, -7.0), p(7.0, -1.0)], stroke);
            painter.rect_stroke(
                egui::Rect::from_min_max(p(-5.0, -1.0), p(5.0, 7.0)),
                egui::CornerRadius::same(2),
                stroke,
                egui::StrokeKind::Inside,
            );
        }
        Screen::Create => {
            painter.circle_stroke(c, 7.0, stroke);
            painter.line_segment([p(-3.2, 0.0), p(3.2, 0.0)], stroke);
            painter.line_segment([p(0.0, -3.2), p(0.0, 3.2)], stroke);
        }
        Screen::Keepalive => {
            painter.circle_stroke(c, 7.0, stroke);
            let pulse = vec![
                p(-4.5, 0.0),
                p(-2.0, 0.0),
                p(-0.8, -3.0),
                p(1.2, 3.0),
                p(2.6, 0.0),
                p(4.5, 0.0),
            ];
            painter.add(egui::Shape::line(pulse, stroke));
        }
        Screen::Cloud => {
            painter.circle_stroke(p(-2.0, -1.5), 3.6, stroke);
            painter.circle_stroke(p(3.0, 0.0), 2.8, stroke);
            painter.line_segment([p(-7.0, 4.0), p(6.0, 4.0)], stroke);
            painter.line_segment([p(-7.0, 4.0), p(-7.0, 1.5)], stroke);
            painter.line_segment([p(6.0, 4.0), p(6.0, 2.0)], stroke);
        }
        Screen::Accounts => {
            painter.circle_stroke(p(0.0, -4.2), 3.0, stroke);
            let shoulders = vec![
                p(-6.0, 7.0),
                p(-3.2, 2.6),
                p(0.0, 1.8),
                p(3.2, 2.6),
                p(6.0, 7.0),
            ];
            painter.add(egui::Shape::line(shoulders, stroke));
        }
        Screen::Settings => {
            painter.circle_stroke(c, 3.0, stroke);
            for step in 0..8 {
                let angle = step as f32 * std::f32::consts::TAU / 8.0;
                let (sin, cos) = angle.sin_cos();
                painter.line_segment(
                    [
                        egui::pos2(c.x + cos * 5.0, c.y + sin * 5.0),
                        egui::pos2(c.x + cos * 7.6, c.y + sin * 7.6),
                    ],
                    stroke,
                );
            }
        }
    }
}

fn stat_card(ui: &mut egui::Ui, label: &str, value: &str, colour: egui::Color32) {
    t::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(t::section_label(label));
        ui.add_space(2.0);
        ui.label(RichText::new(value).size(26.0).color(colour).strong());
    });
}

/// `abcd***xyz` style masking, matching the terminal list.
fn mask(value: &str) -> String {
    let count = value.chars().count();
    if count <= 2 {
        return "*".repeat(count);
    }
    format!(
        "{}{}",
        "*".repeat(count - 2),
        value.chars().skip(count - 2).collect::<String>()
    )
}

fn ui_colour(colour: Color) -> egui::Color32 {
    match colour {
        Color::Warning => t::WARN,
        Color::Fail => t::DANGER,
        Color::Ok | Color::Green => t::OK,
        Color::Cyan => t::ACCENT,
        Color::Blue => t::MUTED,
        _ => t::MUTED,
    }
}

