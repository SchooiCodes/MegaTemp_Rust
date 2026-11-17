//! GUI application shell: navigation rail, top bar and screens.

use eframe::egui::{self, RichText};
use egui_extras::{Column, TableBuilder};

use crate::gui::bridge::{Bridge, CloudFile, Command, UiEvent};
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
            Screen::Dashboard => "Home",
            Screen::Create => "Create",
            Screen::Keepalive => "Alive",
            Screen::Cloud => "Cloud",
            Screen::Accounts => "People",
            Screen::Settings => "Settings",
        }
    }
}

/// One account as the dashboard table needs it.
#[derive(Clone)]
struct Row {
    email: String,
    tags: String,
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
    toast: Option<String>,
    cloud_files: Vec<CloudFile>,
    selected_row: usize,
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
            created: Vec::new(),
            activity: Vec::new(),
            status: None,
            progress: None,
            busy: false,
            prune: false,
            keep_rows: Vec::new(),
            keep_summary: None,
            toast: None,
            cloud_files: Vec::new(),
            selected_row: 0,
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
                    self.toast = Some(format!("Run finished: {ok} created, {failed} failed"));
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
                    self.toast = Some(format!(
                        "Keepalive: {} alive, {} dead",
                        summary.alive, summary.dead
                    ));
                }
                UiEvent::CloudFiles(files) => {
                    self.cloud_files = files;
                }
                UiEvent::Toast { kind, text } => {
                    self.toast = Some(if kind == "error" {
                        format!("Error: {text}")
                    } else {
                        text
                    });
                }
                UiEvent::Idle => {
                    self.busy = false;
                    self.status = None;
                }
            }
        }
    }

    fn start_create(&mut self) {
        self.busy = true;
        self.progress = Some((0, self.create_total, 0, 0));
        self.created.clear();
        self.activity.clear();
        self.toast = None;
        let _ = self.bridge.commands.send(Command::Create {
            total: self.create_total,
            workers: self.create_workers,
        });
    }

    fn start_keepalive(&mut self) {
        self.busy = true;
        self.status = None;
        self.keep_rows.clear();
        self.keep_summary = None;
        self.activity.clear();
        self.toast = None;
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
                    password: account.creds.password,
                    age_days,
                    last_login_days,
                    locked: account.decrypt_failed,
                }
            })
            .collect();
    }

    fn visible_rows(&self) -> Vec<Row> {
        let needle = self.filter.trim().to_lowercase();
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
                        if nav_button(ui, screen, self.screen == screen).clicked() {
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

            let table_height = (ui.available_height() - 76.0).max(180.0);
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
                            ui.label(RichText::new(&account.email).monospace().color(t::TEXT));
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
                        "Register MEGA accounts with disposable mailboxes. Each worker runs the full browser flow.",
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
            t::section_label("Batch");
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Accounts").size(13.0).color(t::MUTED));
                ui.add(
                    egui::DragValue::new(&mut self.create_total)
                        .range(1..=500)
                        .speed(1.0),
                );
                ui.add_space(18.0);
                ui.label(RichText::new("Workers").size(13.0).color(t::MUTED));
                ui.add(
                    egui::DragValue::new(&mut self.create_workers)
                        .range(1..=32)
                        .speed(1.0),
                );
                ui.add_space(18.0);
                ui.label(RichText::new("Provider").size(13.0).color(t::MUTED));
                ui.add(
                    egui::TextEdit::singleline(&mut self.create_provider)
                        .desired_width(150.0)
                        .hint_text("mailtm"),
                );
                ui.add_space(18.0);
                ui.label(RichText::new("Attempts").size(13.0).color(t::MUTED));
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
        });

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
                t::section_label(&format!("Created this run ({})", self.created.len()));
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
                ui.checkbox(&mut self.prune, "Remove dead accounts");
                ui.add_space(12.0);
                if let Some(status) = &self.status {
                    ui.label(RichText::new(status).size(12.0).color(t::MUTED));
                }
            });
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
                        t::section_label(label);
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
            let table_height = (ui.available_height() - 40.0).max(180.0);
            TableBuilder::new(ui)
                .striped(true)
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                .max_scroll_height(table_height)
                .column(Column::initial(300.0).at_least(220.0))
                .column(Column::exact(90.0))
                .column(Column::exact(90.0))
                .column(Column::remainder().at_least(160.0))
                .header(30.0, |mut header| {
                    for title in ["Email", "Status", "Storage", "Detail"] {
                        header.col(|ui| {
                            ui.label(t::section_label(title));
                        });
                    }
                })
                .body(|body| {
                    body.rows(34.0, self.keep_rows.len(), |mut row| {
                        let (email, alive, quota, error) = &self.keep_rows[row.index()];
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
        ui.label(RichText::new("Cloud").size(24.0).strong().color(t::TEXT));
        ui.add_space(4.0);
        ui.label(
            RichText::new("Files on the most recently used account.")
                .size(13.0)
                .color(t::MUTED),
        );
        ui.add_space(18.0);
        ui.horizontal(|ui| {
            if ui.button("Refresh").clicked() && !self.busy {
                self.busy = true;
                let _ = self.bridge.commands.send(Command::ListCloud);
            }
            if ui.button("Upload file").clicked() && !self.busy {
                if let Some(path) = rfd::FileDialog::new().pick_file() {
                    self.busy = true;
                    let _ = self.bridge.commands.send(Command::UploadFile { path });
                }
            }
        });
        if let Some(status) = &self.status {
            ui.add_space(6.0);
            ui.label(RichText::new(status).size(12.0).color(t::MUTED));
        }
        ui.add_space(14.0);
        t::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            if self.cloud_files.is_empty() {
                ui.add_space(10.0);
                ui.label(
                    RichText::new("No files yet. Refresh to list the remote account.")
                        .size(13.0)
                        .color(t::MUTED),
                );
                ui.add_space(10.0);
                return;
            }
            let table_height = (ui.available_height() - 46.0).max(180.0);
            TableBuilder::new(ui)
                .striped(true)
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                .max_scroll_height(table_height)
                .column(Column::initial(260.0).at_least(180.0))
                .column(Column::exact(120.0))
                .column(Column::remainder().at_least(90.0))
                .header(30.0, |mut header| {
                    for title in ["Name", "Handle", "Size"] {
                        header.col(|ui| {
                            ui.label(t::section_label(title));
                        });
                    }
                })
                .body(|mut body| {
                    for file in self.cloud_files.clone() {
                        body.row(34.0, |mut row| {
                            row.col(|ui| { ui.label(&file.name); });
                            row.col(|ui| { ui.monospace(&file.handle); });
                            row.col(|ui| { ui.label(crate::util::human_size(file.size)); });
                        });
                    }
                });
        });
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
            ui.add(egui::TextEdit::singleline(&mut self.filter).hint_text("Filter email or tags"));
            if ui.button("Reload").clicked() {
                self.reload();
            }
        });
        let rows = self.visible_rows();
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
                            self.edit_tags = row.tags.clone();
                            self.edit_notes.clear();
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
                        self.toast = Some("Password copied".to_string());
                    }
                });
                ui.separator();
                ui.label("Tags:");
                ui.text_edit_singleline(&mut self.edit_tags);
                ui.label("Notes:");
                ui.text_edit_multiline(&mut self.edit_notes);
                if ui.button("Save tags").clicked() {
                    // persist via credentials update
                    if let Some(account) = crate::credentials::load_all(&self.cfg.encryption_password, &self.cfg.account_format)
                        .into_iter().find(|a| a.creds.email == row.email) {
                        let mut creds = account.creds;
                        creds.tags = self.edit_tags.clone();
                        creds.notes = self.edit_notes.clone();
                        let _ = crate::credentials::update_credential(&creds.email, &creds, &self.cfg);
                        self.reload();
                        self.toast = Some("Saved".to_string());
                    }
                }
                ui.separator();
                ui.horizontal(|ui| {
                    ui.text_edit_singleline(&mut self.folder_name);
                    if ui.button("Create folder").clicked() && !self.folder_name.trim().is_empty() {
                        let name = self.folder_name.clone();
                        let _ = self.bridge.commands.send(Command::CreateFolder { name });
                        self.folder_name.clear();
                    }
                });
                ui.separator();
                if !self.confirm_delete {
                    if ui.button(egui::RichText::new("Delete account").color(egui::Color32::from_rgb(255, 91, 96))).clicked() {
                        self.confirm_delete = true;
                    }
                } else {
                    ui.label("Are you sure?");
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
            egui::Grid::new("settings_grid").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                ui.label("Max attempts");
                ui.add(egui::DragValue::new(&mut self.settings.max_attempts).range(1..=10));
                ui.end_row();
                ui.label("Email provider");
                ui.text_edit_singleline(&mut self.settings.email_provider);
                ui.end_row();
                ui.label("Mail timeout (s)");
                ui.add(egui::DragValue::new(&mut self.settings.mail_timeout).range(30..=600));
                ui.end_row();
                ui.label("Visible browser");
                ui.checkbox(&mut self.settings.visible_browser, "");
                ui.end_row();
                ui.label("Auto CSV export");
                ui.checkbox(&mut self.settings.csv_export, "");
                ui.end_row();
                ui.label("Auto JSONL export");
                ui.checkbox(&mut self.settings.jsonl_export, "");
                ui.end_row();
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
                ui.label("Account format");
                ui.text_edit_singleline(&mut self.settings.account_format);
                ui.end_row();
                ui.label("Encryption password");
                ui.text_edit_singleline(&mut self.settings.encryption_password);
                ui.end_row();
                ui.label("Chromium path");
                ui.text_edit_singleline(&mut self.settings.executable_path);
                ui.end_row();
            });
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("Save").clicked() {
                    let _ = self.bridge.commands.send(Command::UpdateConfig(self.settings.clone()));
                    self.cfg = self.settings.clone();
                    self.reload();
                }
                if ui.button("Download Chromium").clicked() && !self.busy {
                    self.busy = true;
                    let _ = self.bridge.commands.send(Command::InstallChromium);
                }
            });
            if let Some(toast) = &self.toast {
                ui.label(toast);
            }
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
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.drain_events();
        if self.busy {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(120));
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
                Screen::Accounts => self.accounts_screen(ui),
                Screen::Settings => self.settings_screen(ui),
            });
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

