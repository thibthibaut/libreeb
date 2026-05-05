//! Left information panel with controls.

use crate::state::ViewerState;
use crate::ui::bias_controls::{show_bias_controls, BiasAction};
use crate::ui::playback_controls::show_playback_controls;
use eframe::egui;

pub struct PanelActions {
    pub close: bool,
    pub slice_changed: bool,
    pub bias_changed: bool,
    pub bias_action: BiasAction,
    pub speed_changed: bool,
}

impl Default for PanelActions {
    fn default() -> Self {
        PanelActions {
            close: false,
            slice_changed: false,
            bias_changed: false,
            bias_action: BiasAction::None,
            speed_changed: false,
        }
    }
}

/// Show the left info panel with playback stats and controls.
pub fn show_info_panel(ctx: &egui::Context, v: &mut ViewerState) -> PanelActions {
    let mut actions = PanelActions::default();

    egui::SidePanel::left("info")
        .min_width(220.0)
        .show(ctx, |ui| {
            ui.add_space(6.0);
            ui.label(egui::RichText::new(&v.label).strong());
            ui.label(format!("{}×{}", v.width, v.height));
            ui.separator();

            // Basic playback stats
            ui.label(egui::RichText::new("Stats").strong());
            ui.label(format!("Frame:        #{}", v.frame_count));
            ui.label(format!("Events/slice:  {}", v.last_ev_count));
            let rate = v.last_rate;
            ui.label(if rate >= 1_000_000.0 {
                format!("Event rate:    {:.1} Mev/s", rate / 1_000_000.0)
            } else if rate >= 1_000.0 {
                format!("Event rate:    {:.1} kev/s", rate / 1_000.0)
            } else {
                format!("Event rate:    {:.0} ev/s", rate)
            });

            // Slice window control
            ui.add_space(4.0);
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

            // Playback speed controls (file mode only)
            if show_playback_controls(ui, &mut v.playback_state, v.is_camera) {
                actions.speed_changed = true;
            }

            // Bias controls (camera mode only)
            if v.is_camera && !v.bias_defaults.is_empty() {
                let bias_action = show_bias_controls(ui, &v.bias_defaults, &mut v.bias_offsets);
                if bias_action != BiasAction::None {
                    actions.bias_action = bias_action;
                    if bias_action == BiasAction::Reset {
                        actions.bias_changed = true;
                    }
                }
            } else if !v.header_info.is_empty() {
                // File header info (file mode only)
                ui.separator();
                ui.label(egui::RichText::new("File header").strong());
                egui::ScrollArea::vertical()
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

            // Close button at bottom
            ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                ui.add_space(6.0);
                if ui.button("Close source").clicked() {
                    actions.close = true;
                }
            });
        });

    actions
}
