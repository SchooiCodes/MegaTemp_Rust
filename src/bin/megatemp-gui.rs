//! MegaTemp GUI entry point.
//!
//! Hide the console on Windows in every profile, not just release, so the
//! desktop app always looks like a native application.
#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use megatemp::gui::{egui, App};

fn main() -> eframe::Result<()> {
    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../../assets/megatemp.png"))
        .expect("the bundled MegaTemp logo is a valid PNG");
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("MegaTemp")
            .with_icon(icon)
            .with_inner_size([1180.0, 760.0])
            .with_min_inner_size([920.0, 620.0]),
        ..Default::default()
    };
    eframe::run_native("MegaTemp", options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}
