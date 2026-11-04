// Hide the console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;

use eframe::egui;
use pdf_splitter::app::SplitterApp;

fn main() -> eframe::Result {
    // A PDF dropped onto the .exe (or passed on the command line) is opened directly.
    let initial = std::env::args_os().nth(1).map(PathBuf::from);

    let mut viewport = egui::ViewportBuilder::default()
        .with_title("PDF Splitter")
        .with_inner_size([860.0, 680.0])
        .with_min_inner_size([680.0, 560.0])
        .with_drag_and_drop(true);
    if let Ok(icon) = eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png")) {
        viewport = viewport.with_icon(icon);
    }

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "PDF Splitter",
        options,
        Box::new(move |cc| Ok(Box::new(SplitterApp::new(cc, initial)))),
    )
}
