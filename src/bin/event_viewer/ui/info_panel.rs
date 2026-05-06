//! Left information panel with controls.

use crate::state::ViewerState;
use crate::ui::bias_controls::{show_bias_controls, BiasAction};
use crate::ui::recording_controls::show_recording_controls;
use eframe::egui;

pub struct PanelActions {
    pub close: bool,
    pub slice_changed: bool,
    pub bias_changed: bool,
    pub bias_action: BiasAction,
    pub speed_changed: bool,
    pub recording_start: bool,
    pub recording_stop: bool,
}

impl Default for PanelActions {
    fn default() -> Self {
        PanelActions {
            close: false,
            slice_changed: false,
            bias_changed: false,
            bias_action: BiasAction::None,
            speed_changed: false,
            recording_start: false,
            recording_stop: false,
        }
    }
}

/// Format microseconds as HH:MM:SS.mmm
fn format_timestamp(us: u64) -> String {
    let total_ms = us / 1000;
    let ms = total_ms % 1000;
    let total_s = total_ms / 1000;
    let s = total_s % 60;
    let total_m = total_s / 60;
    let m = total_m % 60;
    let h = total_m / 60;
    format!("{:02}:{:02}:{:02}.{:03}", h, m, s, ms)
}

/// Show the left info panel with playback stats and controls.
pub fn show_info_panel(ctx: &egui::Context, v: &mut ViewerState) -> PanelActions {
    let mut actions = PanelActions::default();

    egui::SidePanel::left("info")
        .min_width(250.0)
        .show(ctx, |ui| {
            ui.add_space(6.0);

            // ═══════════════════════════════════════════════════
            // File Info (collapsible)
            // ═══════════════════════════════════════════════════
            egui::CollapsingHeader::new("File Info")
                .default_open(true)
                .show(ui, |ui| {
                    ui.add_space(2.0);
                    ui.label(egui::RichText::new(&v.label).strong());
                    ui.label(format!("{}×{}", v.width, v.height));

                    // File header (file mode only)
                    if !v.is_camera && !v.header_info.is_empty() {
                        ui.add_space(4.0);
                        ui.label(egui::RichText::new("Header").strong().small());
                        egui::ScrollArea::vertical()
                            .max_height(200.0)
                            .id_salt("hdr_scroll")
                            .show(ui, |ui| {
                                egui::Grid::new("hdr")
                                    .num_columns(2)
                                    .striped(true)
                                    .show(ui, |ui| {
                                        for (k, val) in &v.header_info {
                                            ui.label(egui::RichText::new(k).monospace().small());
                                            ui.label(egui::RichText::new(val).monospace().small());
                                            ui.end_row();
                                        }
                                    });
                            });
                    }

                    ui.add_space(4.0);
                    if ui.button("Close source").clicked() {
                        actions.close = true;
                    }
                });

            ui.add_space(4.0);

            // ═══════════════════════════════════════════════════
            // Stats (collapsible)
            // ═══════════════════════════════════════════════════
            egui::CollapsingHeader::new("Stats")
                .default_open(true)
                .show(ui, |ui| {
                    ui.add_space(2.0);

                    ui.label(format!("Frame:     #{}", v.frame_count));

                    // Event rate
                    let rate = v.last_rate;
                    if rate >= 1_000_000.0 {
                        ui.label(format!("Rate:      {:.2} Mev/s", rate / 1_000_000.0));
                    } else if rate >= 1_000.0 {
                        ui.label(format!("Rate:      {:.2} kev/s", rate / 1_000.0));
                    } else {
                        ui.label(format!("Rate:      {:.0} ev/s", rate));
                    }

                    // Absolute timestamp
                    if let Some(ts) = v.last_timestamp {
                        ui.label(format!("Time:      {}", format_timestamp(ts)));
                    }

                    // Relative timestamp
                    if let (Some(first), Some(last)) = (v.first_timestamp, v.last_timestamp) {
                        let elapsed_us = last.saturating_sub(first);
                        ui.label(format!("Elapsed:   {}", format_timestamp(elapsed_us)));
                    }
                });

            ui.add_space(4.0);

            // ═══════════════════════════════════════════════════
            // Playback Controls (collapsible)
            // ═══════════════════════════════════════════════════
            egui::CollapsingHeader::new("Playback Controls")
                .default_open(true)
                .show(ui, |ui| {
                    ui.add_space(2.0);

                    // Slice window control
                    ui.label("Slice window (µs):");
                    if ui
                        .add(
                            egui::Slider::new(&mut v.window_us, 1_000.0..=200_000.0)
                                .step_by(1_000.0)
                                .integer(),
                        )
                        .changed()
                    {
                        actions.slice_changed = true;
                    }

                    // Speed dropdown + FPS (file mode only)
                    if !v.is_camera {
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
                            .find(|(_, s)| (*s - v.playback_state.speed_multiplier).abs() < 0.0001)
                            .map(|(l, _)| *l)
                            .unwrap_or("Custom");

                        egui::ComboBox::from_label("Speed")
                            .selected_text(current_label)
                            .show_ui(ui, |ui| {
                                for (label, speed) in speed_options {
                                    if ui
                                        .selectable_value(&mut v.playback_state.speed_multiplier, speed, label)
                                        .clicked()
                                    {
                                        actions.speed_changed = true;
                                    }
                                }
                            });

                        ui.add_space(4.0);
                        ui.label(format!("FPS: {:.1}", v.playback_state.actual_fps));
                    }
                });

            ui.add_space(4.0);

            // ═══════════════════════════════════════════════════
            // Recording (collapsible)
            // ═══════════════════════════════════════════════════
            egui::CollapsingHeader::new("Recording")
                .default_open(true)
                .show(ui, |ui| {
                    let rec_actions = show_recording_controls(ui, &v.recorder);
                    if rec_actions.start {
                        actions.recording_start = true;
                    }
                    if rec_actions.stop {
                        actions.recording_stop = true;
                    }
                });

            ui.add_space(4.0);

            // ═══════════════════════════════════════════════════
            // Bias Controls (camera mode only, collapsible)
            // ═══════════════════════════════════════════════════
            if v.is_camera && !v.bias_defaults.is_empty() {
                egui::CollapsingHeader::new("Biases")
                    .default_open(true)
                    .show(ui, |ui| {
                        ui.add_space(2.0);
                        let result = show_bias_controls(ui, &v.bias_defaults, &mut v.bias_offsets);
                        if result.action != BiasAction::None {
                            actions.bias_action = result.action;
                        }
                        if result.changed || result.action == BiasAction::Reset {
                            actions.bias_changed = true;
                        }
                    });
            }
        });

    actions
}
