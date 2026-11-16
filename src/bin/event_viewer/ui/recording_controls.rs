//! Recording controls UI.

use crate::recording::RecordingState;
use eframe::egui;
use std::sync::{Arc, Mutex};

pub struct RecordingActions {
    pub start: bool,
    pub stop: bool,
}

impl Default for RecordingActions {
    fn default() -> Self {
        RecordingActions {
            start: false,
            stop: false,
        }
    }
}

/// Show recording controls (filename input + start/stop button).
pub fn show_recording_controls(
    ui: &mut egui::Ui,
    recorder: &Arc<Mutex<RecordingState>>,
) -> RecordingActions {
    let mut actions = RecordingActions::default();

    if let Ok(mut rec) = recorder.lock() {
        ui.add_space(2.0);

        if rec.is_recording {
            // Recording in progress - show status and stop button
            ui.label(
                egui::RichText::new("🔴 Recording...").color(egui::Color32::from_rgb(220, 50, 50)),
            );

            ui.add_space(4.0);
            ui.label(format!("Buffer: {:.2} MB", rec.buffer_size_mb()));

            // Warn if buffer is getting large
            if rec.buffer_size_mb() > 500.0 {
                ui.colored_label(
                    egui::Color32::from_rgb(255, 150, 0),
                    "⚠ Large buffer! Consider stopping.",
                );
            }

            ui.add_space(4.0);
            if ui.button("⏹ Stop Recording").clicked() {
                actions.stop = true;
            }
        } else {
            // Not recording - show filename input and start button
            ui.label("Filename:");
            ui.text_edit_singleline(&mut rec.filename);

            ui.add_space(4.0);
            if ui.button("⏺ Start Recording").clicked() {
                actions.start = true;
            }
        }
    }

    actions
}
