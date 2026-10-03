#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod backend;
mod frame;
mod launch;
mod session;
mod settings;
mod sshconfig;
mod terminal;
mod ui;

fn main() -> eframe::Result {
    let icon = egui::IconData {
        rgba: include_bytes!("../assets/icon-64.rgba").to_vec(),
        width: 64,
        height: 64,
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("IkTerminal")
            .with_app_id("IkTerminal")
            .with_inner_size([1100.0, 700.0])
            .with_min_inner_size([480.0, 300.0])
            .with_decorations(false)
            .with_icon(icon),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "IkTerminal",
        options,
        Box::new(|cc| {
            let (directory, notice) = match launch::from_args() {
                Ok(dir) => (dir, None),
                Err(error) => (None, Some(error)),
            };
            Ok(Box::new(app::App::new(cc, directory, notice)))
        }),
    )
}
