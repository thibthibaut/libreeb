//! Bias control UI with save/load functionality.

use eframe::egui;
use libreeb::biases::{BiasGroup, BIAS_TABLE};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BiasAction {
    None,
    Save,
    Load,
    Reset,
}

/// Parse bias file content into offset vector.
pub fn parse_bias_file(content: &str) -> Result<Vec<i32>, String> {
    let mut offsets = vec![0i32; BIAS_TABLE.len()];

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        // Parse: "0    % bias_diff"
        let parts: Vec<&str> = line.split('%').collect();
        if parts.len() != 2 {
            return Err(format!("Invalid line format: {}", line));
        }

        let offset: i32 = parts[0]
            .trim()
            .parse()
            .map_err(|_| format!("Invalid offset: {}", parts[0].trim()))?;

        let name = parts[1].trim();

        // Find bias by name
        let idx = BIAS_TABLE
            .iter()
            .position(|b| b.name == name)
            .ok_or_else(|| format!("Unknown bias: {}", name))?;

        offsets[idx] = offset;
    }

    Ok(offsets)
}

/// Format bias offsets into .bias file content.
pub fn format_bias_file(offsets: &[i32]) -> String {
    BIAS_TABLE
        .iter()
        .enumerate()
        .map(|(i, bi)| {
            let offset = offsets.get(i).copied().unwrap_or(0);
            format!("{}    % {}\n", offset, bi.name)
        })
        .collect()
}

/// Show bias controls with save/load/reset buttons and sliders.
pub fn show_bias_controls(
    ui: &mut egui::Ui,
    bias_defaults: &[u8],
    bias_offsets: &mut Vec<i32>,
) -> BiasAction {
    let mut action = BiasAction::None;

    ui.separator();

    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Biases").strong());

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .small_button("💾 Save")
                .on_hover_text("Save biases to .bias file")
                .clicked()
            {
                action = BiasAction::Save;
            }
            if ui
                .small_button("📂 Load")
                .on_hover_text("Load biases from .bias file")
                .clicked()
            {
                action = BiasAction::Load;
            }
            if ui
                .small_button("🔄 Reset")
                .on_hover_text("Reset all offsets to defaults")
                .clicked()
            {
                action = BiasAction::Reset;
            }
        });
    });

    // Bias sliders grouped by category
    egui::ScrollArea::vertical()
        .id_salt("bias_scroll")
        .show(ui, |ui| {
            for (group_label, group_variant) in &[
                ("Bandwidth", BiasGroup::Bandwidth),
                ("Contrast", BiasGroup::Contrast),
                ("Advanced", BiasGroup::Advanced),
            ] {
                egui::CollapsingHeader::new(egui::RichText::new(*group_label).small().strong())
                    .default_open(true)
                    .show(ui, |ui| {
                        egui::Grid::new(format!("bias_{}", group_label))
                            .num_columns(2)
                            .spacing([6.0, 3.0])
                            .show(ui, |ui| {
                                for (i, bi) in BIAS_TABLE.iter().enumerate() {
                                    if bi.group != *group_variant {
                                        continue;
                                    }

                                    if let Some(offset) = bias_offsets.get_mut(i) {
                                        let factory = bias_defaults.get(i).copied().unwrap_or(128);
                                        let abs_val = (factory as i32 + *offset).clamp(0, 255);

                                        ui.label(
                                            egui::RichText::new(bi.name).monospace().small(),
                                        )
                                        .on_hover_text(bi.hint);

                                        ui.label(
                                            egui::RichText::new(format!("{}", abs_val))
                                                .monospace()
                                                .small(),
                                        );
                                        ui.end_row();

                                        ui.label("");
                                        ui.add(
                                            egui::Slider::new(offset, bi.rec_min..=bi.rec_max)
                                                .show_value(false),
                                        );
                                        ui.end_row();
                                    }
                                }
                            });
                    });
            }
        });

    action
}
