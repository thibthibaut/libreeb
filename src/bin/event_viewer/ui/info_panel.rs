//! Left information panel with controls.

use crate::state::{DisplayMode, ViewerState};
use crate::ui::bias_controls::{show_bias_controls, BiasAction};
use crate::ui::recording_controls::show_recording_controls;
use eframe::egui;

// ── Action bundle returned to main ───────────────────────────────────────────

pub struct PanelActions {
    pub close: bool,
    pub pause_changed: bool,
    pub bias_changed: bool,
    pub bias_action: BiasAction,
    pub speed_changed: bool,
    pub recording_start: bool,
    pub recording_stop: bool,
    /// ROI settings changed — apply to hardware.
    pub roi_changed: bool,
}

impl Default for PanelActions {
    fn default() -> Self {
        PanelActions {
            close: false,
            pause_changed: false,
            bias_changed: false,
            bias_action: BiasAction::None,
            speed_changed: false,
            recording_start: false,
            recording_stop: false,
            roi_changed: false,
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn format_timestamp(us: u64) -> String {
    let total_ms = us / 1000;
    let ms = total_ms % 1000;
    let s = (total_ms / 1000) % 60;
    let m = (total_ms / 60_000) % 60;
    let h = total_ms / 3_600_000;
    format!("{:02}:{:02}:{:02}.{:03}", h, m, s, ms)
}

// ── Panel ─────────────────────────────────────────────────────────────────────

pub fn show_info_panel(ctx: &egui::Context, v: &mut ViewerState) -> PanelActions {
    let mut actions = PanelActions::default();

    egui::SidePanel::left("info")
        .min_width(260.0)
        .show(ctx, |ui| {
            ui.add_space(6.0);

            // ═══ File Info ════════════════════════════════════════════════════
            egui::CollapsingHeader::new("File Info")
                .default_open(true)
                .show(ui, |ui| {
                    ui.label(egui::RichText::new(&v.label).strong());
                    ui.label(format!("{}×{}", v.width, v.height));
                    if !v.is_camera && !v.header_info.is_empty() {
                        ui.add_space(4.0);
                        egui::ScrollArea::vertical()
                            .max_height(160.0)
                            .id_salt("hdr")
                            .show(ui, |ui| {
                                egui::Grid::new("hdr_grid")
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

            // ═══ Stats ════════════════════════════════════════════════════════
            egui::CollapsingHeader::new("Stats")
                .default_open(true)
                .show(ui, |ui| {
                    ui.label(format!("Frame:  #{}", v.frame_count));
                    let rate = v.rate_history.newest().map(|s| s.rate).unwrap_or(0.0);
                    if rate >= 1_000_000.0 {
                        ui.label(format!("Rate:   {:.2} Mev/s", rate / 1e6));
                    } else if rate >= 1_000.0 {
                        ui.label(format!("Rate:   {:.2} kev/s", rate / 1e3));
                    } else {
                        ui.label(format!("Rate:   {:.0} ev/s", rate));
                    }
                    if let Some(ts) = v.last_timestamp {
                        ui.label(format!("Time:   {}", format_timestamp(ts)));
                    }
                    if let (Some(first), Some(last)) = (v.first_timestamp, v.last_timestamp) {
                        ui.label(format!(
                            "Elapsed: {}",
                            format_timestamp(last.saturating_sub(first))
                        ));
                    }
                    ui.label(format!("Render: {:.1} fps", v.render_fps));
                    if v.dropped_count > 0 {
                        ui.colored_label(
                            egui::Color32::from_rgb(235, 160, 50),
                            format!("Dropped: {} batches (buffer full)", v.dropped_count),
                        );
                    }
                });

            ui.add_space(4.0);

            // ═══ Visualisation ════════════════════════════════════════════════
            egui::CollapsingHeader::new("Visualisation")
                .default_open(true)
                .show(ui, |ui| {
                    ui.label("Canvas:");
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut v.display_mode, DisplayMode::Normal, "Normal");
                        ui.selectable_value(
                            &mut v.display_mode,
                            DisplayMode::TimeSurface,
                            "Time Surface",
                        );
                        ui.selectable_value(&mut v.display_mode, DisplayMode::Xyt3D, "3D XYT");
                    });

                    if v.display_mode == DisplayMode::Normal {
                        ui.add_space(4.0);
                        let mut accumulation_ms = v.window_us / 1_000.0;
                        if ui
                            .add(
                                egui::Slider::new(&mut accumulation_ms, 1.0..=200.0)
                                    .text("accumulation")
                                    .suffix(" ms"),
                            )
                            .changed()
                        {
                            v.window_us = accumulation_ms * 1_000.0;
                        }
                    } else if v.display_mode == DisplayMode::TimeSurface {
                        ui.add_space(4.0);
                        let mut decay_ms = v.ts_decay_us as f32 / 1_000.0;
                        if ui
                            .add(
                                egui::Slider::new(&mut decay_ms, 1.0..=500.0)
                                    .text("decay")
                                    .suffix(" ms"),
                            )
                            .changed()
                        {
                            v.ts_decay_us = (decay_ms * 1_000.0) as u64;
                        }
                    } else if v.display_mode == DisplayMode::Xyt3D {
                        ui.add_space(4.0);
                        ui.add(
                            egui::Slider::new(&mut v.xyt_point_size, 0.5..=4.0)
                                .text("pt size")
                                .suffix(" px"),
                        );
                    }
                });

            ui.add_space(4.0);

            // ═══ ROI (camera only) ════════════════════════════════════════════
            if v.is_camera {
                egui::CollapsingHeader::new("ROI")
                    .default_open(true)
                    .show(ui, |ui| {
                        let prev_enabled = v.roi_enabled;
                        ui.checkbox(&mut v.roi_enabled, "Enable hardware ROI");
                        if v.roi_enabled != prev_enabled {
                            actions.roi_changed = true;
                        }

                        if v.roi_enabled {
                            let w = v.width;
                            let h = v.height;

                            let prev = v.roi_left;
                            let max_left = w.saturating_sub(v.roi_width).saturating_sub(1);
                            ui.add(egui::Slider::new(&mut v.roi_left, 0..=max_left).text("left"));
                            if v.roi_left != prev {
                                actions.roi_changed = true;
                            }

                            let prev = v.roi_top;
                            let max_top = h.saturating_sub(v.roi_height).saturating_sub(1);
                            ui.add(egui::Slider::new(&mut v.roi_top, 0..=max_top).text("top"));
                            if v.roi_top != prev {
                                actions.roi_changed = true;
                            }

                            let prev = v.roi_width;
                            let max_w = w.saturating_sub(v.roi_left);
                            ui.add(egui::Slider::new(&mut v.roi_width, 1..=max_w).text("width"));
                            if v.roi_width != prev {
                                actions.roi_changed = true;
                            }

                            let prev = v.roi_height;
                            let max_h = h.saturating_sub(v.roi_top);
                            ui.add(egui::Slider::new(&mut v.roi_height, 1..=max_h).text("height"));
                            if v.roi_height != prev {
                                actions.roi_changed = true;
                            }

                            ui.label(
                                egui::RichText::new(format!(
                                    "→ ({},{})–({},{})  {}×{}",
                                    v.roi_left,
                                    v.roi_top,
                                    v.roi_left + v.roi_width - 1,
                                    v.roi_top + v.roi_height - 1,
                                    v.roi_width,
                                    v.roi_height,
                                ))
                                .small()
                                .color(egui::Color32::GRAY),
                            );
                        }
                    });
                ui.add_space(4.0);
            }

            // ═══ Playback Controls ════════════════════════════════════════════
            egui::CollapsingHeader::new("Playback Controls")
                .default_open(true)
                .show(ui, |ui| {
                    ui.label(if v.is_camera {
                        "Live replay"
                    } else {
                        "File playback"
                    });
                    let label = if v.playback_paused { "Play" } else { "Pause" };
                    if ui.button(label).clicked() {
                        v.playback_paused = !v.playback_paused;
                        actions.pause_changed = true;
                    }

                    ui.add_space(4.0);
                    let speed_opts = [
                        ("×0.001", 0.001_f32),
                        ("×0.003", 0.003),
                        ("×0.01", 0.01),
                        ("×0.1", 0.1),
                        ("×0.5", 0.5),
                        ("×1", 1.0),
                        ("×2", 2.0),
                        ("×3", 3.0),
                    ];
                    let cur = speed_opts
                        .iter()
                        .find(|(_, s)| (*s - v.playback_speed).abs() < 0.0001)
                        .map(|(l, _)| *l)
                        .unwrap_or("Custom");
                    egui::ComboBox::from_label("Speed")
                        .selected_text(cur)
                        .show_ui(ui, |ui| {
                            for (label, speed) in speed_opts {
                                if ui
                                    .selectable_value(&mut v.playback_speed, speed, label)
                                    .clicked()
                                {
                                    actions.speed_changed = true;
                                }
                            }
                        });
                    ui.label(format!("FPS: {:.1}", v.render_fps));
                });

            ui.add_space(4.0);

            // ═══ Recording ════════════════════════════════════════════════════
            egui::CollapsingHeader::new("Recording")
                .default_open(true)
                .show(ui, |ui| {
                    let ra = show_recording_controls(ui, &v.recorder);
                    if ra.start {
                        actions.recording_start = true;
                    }
                    if ra.stop {
                        actions.recording_stop = true;
                    }
                });

            ui.add_space(4.0);

            // ═══ Biases (camera only) ═════════════════════════════════════════
            if v.is_camera && !v.bias_defaults.is_empty() {
                egui::CollapsingHeader::new("Biases")
                    .default_open(false)
                    .show(ui, |ui| {
                        let res = show_bias_controls(ui, &v.bias_defaults, &mut v.bias_offsets);
                        if res.action != BiasAction::None {
                            actions.bias_action = res.action;
                        }
                        if res.changed || res.action == BiasAction::Reset {
                            actions.bias_changed = true;
                        }
                    });
            }
        });

    actions
}
