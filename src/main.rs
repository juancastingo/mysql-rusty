mod ai;
mod app;
mod config;
mod data_diff;
mod db;
mod demo;
mod er_diagram;
mod export;
mod schema_diff;
mod syntax;
mod table_designer;
#[cfg(test)]
pub mod test_fixtures;

use app::MysqlRustyApp;
use std::sync::Arc;
use tokio::runtime::Runtime;

fn main() -> eframe::Result<()> {
    let rt = Arc::new(
        Runtime::new().expect("Failed to initialize asynchronous Tokio runtime for MySQL-Rusty"),
    );

    let mut viewport = egui::ViewportBuilder::default()
        .with_title("MySQL-Rusty Workbench — Native Rust SQL Client & AI Copilot")
        .with_inner_size([1420.0, 860.0])
        .with_min_inner_size([960.0, 620.0])
        .with_app_id("mysql-rusty");

    if let Ok(icon) = eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png")) {
        viewport = viewport.with_icon(Arc::new(icon));
    }

    let native_options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "mysql-rusty",
        native_options,
        Box::new(move |cc| Ok(Box::new(MysqlRustyApp::new(cc, rt.clone())))),
    )
}
