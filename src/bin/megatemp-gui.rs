//! MegaTemp GUI entry point.

#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use megatemp::gui::{egui, App};

/// The MegaTemp logo, decoded from the embedded PNG for the window/taskbar icon.
fn logo_icon() -> egui::IconData {
    let image = image::load_from_memory(include_bytes!("../../assets/MegaTemp.png"))
        .expect("embedded MegaTemp logo is a valid PNG")
        .to_rgba8();
    let (width, height) = image.dimensions();
    egui::IconData {
        rgba: image.into_raw(),
        width,
        height,
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("MegaTemp")
            .with_icon(logo_icon())
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
