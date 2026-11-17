//! Native GUI shell built on egui/eframe.

pub mod app;
pub mod bridge;
pub mod theme;

pub use app::App;
pub use eframe::egui;
