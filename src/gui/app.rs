//! GUI application shell: navigation rail, top bar and screens.

use eframe::egui::{self, RichText};
use egui_extras::{Column, TableBuilder};

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
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        t::apply(&cc.egui_ctx);
        let cfg = config::load(None);
        let mut app = Self {
            cfg,
            screen: Screen::Dashboard,
            rows: Vec::new(),
            filter: String::new(),
        };
        app.reload();
        app
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
                            t::MUTED,
                        );
                        painter.text(
                            egui::pos2(rect.left() + 27.0, rect.center().y),
                            egui::Align2::LEFT_CENTER,
                            "Idle",
                            egui::FontId::proportional(12.0),
                            t::MUTED,
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
                ui.label(RichText::new("Accounts").heading().strong());
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
        ui.horizontal(|ui| {
            stat_card(ui, "Saved accounts", &total.to_string(), t::TEXT);
            stat_card(ui, "Tagged", &with_tags.to_string(), t::TEXT);
            stat_card(ui, "Locked files", &locked.to_string(), if locked > 0 { t::WARN } else { t::MUTED });
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
                Screen::Create => self.placeholder(
                    ui,
                    "Create accounts",
                    "Single, sequential and parallel batches with live worker progress.",
                ),
                Screen::Keepalive => self.placeholder(
                    ui,
                    "Keepalive",
                    "Log every saved account in, read storage and prune dead ones.",
                ),
                Screen::Cloud => self.placeholder(
                    ui,
                    "Cloud",
                    "Browse, upload and download files on any saved account.",
                ),
                Screen::Accounts => self.placeholder(
                    ui,
                    "Accounts",
                    "Reveal passwords, edit tags and notes, or remove an account.",
                ),
                Screen::Settings => self.placeholder(
                    ui,
                    "Settings",
                    "Attempts, provider, browser, proxies, exports and encryption.",
                ),
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
        ui.set_width(200.0);
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
