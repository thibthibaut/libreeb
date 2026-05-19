//! Initial file/camera selection screen.

use eframe::egui;
use libreeb::RawEventType;

pub enum SelectionAction {
    None,
    OpenFile,
    OpenCamera(RawEventType),
}

/// Show the initial selection screen.
pub fn show_selection_screen(
    ctx: &egui::Context,
    camera_event_type: &mut RawEventType,
) -> SelectionAction {
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
            ui.horizontal(|ui| {
                ui.label("Format:");
                ui.selectable_value(camera_event_type, RawEventType::Evt21, "EVT21");
                ui.selectable_value(camera_event_type, RawEventType::Evt3, "EVT3");
            });
            ui.add_space(6.0);
            if ui.button("  Connect EVK4…  ").clicked() {
                action = SelectionAction::OpenCamera(*camera_event_type);
            }
        });
    });

    action
}
