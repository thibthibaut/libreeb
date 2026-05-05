//! Center canvas for displaying event frames.

use crate::state::ViewerState;
use eframe::egui;

/// Show the center canvas with the current frame.
pub fn show_canvas(ctx: &egui::Context, v: &ViewerState) {
    egui::CentralPanel::default().show(ctx, |ui| {
        if let Some(tex) = &v.texture {
            let avail = ui.available_size();
            let aspect = v.width as f32 / v.height as f32;
            let size = if avail.x / avail.y > aspect {
                egui::vec2(avail.y * aspect, avail.y)
            } else {
                egui::vec2(avail.x, avail.x / aspect)
            };
            ui.centered_and_justified(|ui| {
                ui.image((tex.id(), size));
            });
        } else {
            ui.centered_and_justified(|ui| {
                ui.label("Waiting for first frame…");
            });
        }
    });
}
