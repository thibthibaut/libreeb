//! Initial file/camera selection screen.

use eframe::egui;

pub enum SelectionAction {
    None,
    OpenFile,
    OpenCamera,
}

/// Show the initial selection screen.
pub fn show_selection_screen(ctx: &egui::Context) -> SelectionAction {
    let mut action = SelectionAction::None;

    egui::CentralPanel::default().show(ctx, |ui| {
        ui.add_space(40.0);
        ui.vertical_centered(|ui| {
            ui.heading("libreeb Event Viewer");
            ui.add_space(20.0);
            if ui.button("  Open RAW file…  ").clicked() {
                action = SelectionAction::OpenFile;
            }
            ui.add_space(10.0);
            ui.separator();
            ui.add_space(10.0);
            ui.label("Live camera (Chromium + WebUSB required):");
            ui.add_space(6.0);
            if ui.button("  Connect EVK4…  ").clicked() {
                action = SelectionAction::OpenCamera;
            }
        });
    });

    action
}
