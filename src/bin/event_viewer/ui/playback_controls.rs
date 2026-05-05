//! Playback speed and FPS display controls.

use crate::state::PlaybackState;
use eframe::egui;

/// Show playback controls (speed dropdown + FPS display).
/// Returns true if playback speed changed.
pub fn show_playback_controls(
    ui: &mut egui::Ui,
    state: &mut PlaybackState,
    is_camera: bool,
) -> bool {
    let mut changed = false;

    ui.separator();
    ui.label(egui::RichText::new("Playback").strong());

    // Speed dropdown (file mode only)
    if !is_camera {
        ui.add_space(4.0);

        let speed_options = [
            ("×0.003 (slowest)", 0.003),
            ("×0.01", 0.01),
            ("×0.1", 0.1),
            ("×0.5", 0.5),
            ("×1 (real-time)", 1.0),
            ("×2", 2.0),
            ("×3 (fastest)", 3.0),
        ];

        let current_label = speed_options
            .iter()
            .find(|(_, s)| (*s - state.speed_multiplier).abs() < 0.0001)
            .map(|(l, _)| *l)
            .unwrap_or("Custom");

        egui::ComboBox::from_label("Speed")
            .selected_text(current_label)
            .show_ui(ui, |ui| {
                for (label, speed) in speed_options {
                    if ui
                        .selectable_value(&mut state.speed_multiplier, speed, label)
                        .clicked()
                    {
                        state.target_fps = 30.0 * speed;
                        changed = true;
                    }
                }
            });
    }

    // FPS display (always visible)
    ui.add_space(4.0);
    ui.label(format!("Target FPS: {:.1}", state.target_fps));

    let fps_color = if state.actual_fps < state.target_fps * 0.6 {
        egui::Color32::RED // Struggling
    } else if state.actual_fps < state.target_fps * 0.8 {
        egui::Color32::YELLOW // Close
    } else {
        ui.style().visuals.text_color() // Good
    };

    ui.colored_label(fps_color, format!("Actual FPS: {:.1}", state.actual_fps));

    if state.actual_fps < state.target_fps * 0.6 {
        ui.colored_label(egui::Color32::RED, "⚠ Display struggling");
    }

    changed
}
