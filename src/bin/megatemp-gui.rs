//! MegaTemp GUI entry point.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use megatemp::gui::{egui, App};

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("MegaTemp")
            .with_inner_size([1180.0, 760.0])
            .with_min_inner_size([920.0, 620.0]),
        ..Default::default()
    };
    eframe::run_native(
        "MegaTemp",
        options,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}
