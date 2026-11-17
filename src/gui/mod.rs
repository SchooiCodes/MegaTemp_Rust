//! Native GUI shell built on egui/eframe.
pub mod app;
pub mod bridge;
pub mod theme;
pub use app::App;
pub use eframe::egui;

/// The MegaTemp logo, decoded from the embedded PNG for the window/taskbar icon.
fn logo_icon() -> egui::IconData {
    let image = image::load_from_memory(include_bytes!("../../assets/megatemp.png"))
        .expect("embedded MegaTemp logo is a valid PNG")
        .to_rgba8();
    let (width, height) = image.dimensions();
    egui::IconData {
        rgba: image.into_raw(),
        width,
        height,
    }
}

/// Launch the native GUI. The CLI entry point calls this when no arguments
/// (or `--gui`) are supplied, so the graphical app is the default `megatemp`.
pub fn launch() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("MegaTemp")
            .with_icon(logo_icon())
            .with_inner_size([1360.0, 880.0])
            .with_min_inner_size([820.0, 560.0])
            .with_maximized(true),
        ..Default::default()
    };
    eframe::run_native(
        "MegaTemp",
        options,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}
