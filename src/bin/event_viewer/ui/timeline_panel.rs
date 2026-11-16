//! Bottom panel: scrolling event-rate plot.

use crate::state::ViewerState;
use eframe::egui;
use egui_plot::{Legend, Line, Plot, PlotBounds, PlotPoints};

// ── Public entry point ────────────────────────────────────────────────────────

pub fn show_timeline_panel(ctx: &egui::Context, v: &mut ViewerState) {
    egui::TopBottomPanel::bottom("timeline")
        .min_height(160.0)
        .resizable(true)
        .show(ctx, |ui| {
            // ── Header row ────────────────────────────────────────────────────
            ui.horizontal(|ui| {
                ui.strong("Event Rate");
                ui.separator();
                // rate_lookback_s is display-only; changing it never truncates
                // stored samples — the TimeWindow retains 5 minutes regardless.
                ui.add(
                    egui::Slider::new(&mut v.rate_lookback_s, 1.0..=60.0)
                        .suffix(" s")
                        .text("lookback"),
                );
            });
            ui.separator();

            let avail = ui.available_size();
            show_rate_plot(ui, v, avail);
        });
}

// ── Scrolling event-rate plot ─────────────────────────────────────────────────

fn show_rate_plot(ui: &mut egui::Ui, v: &ViewerState, avail: egui::Vec2) {
    let newest_t_us = v.rate_history.newest_t().unwrap_or(0);
    let lookback_us = (v.rate_lookback_s * 1_000_000.0) as u64;
    let t_min_us = newest_t_us.saturating_sub(lookback_us);

    let t_min_s = t_min_us as f64 * 1e-6;
    let t_max_s = newest_t_us as f64 * 1e-6;

    let points: PlotPoints = v
        .rate_history
        .iter()
        .filter(|s| s.t >= t_min_us)
        .map(|s| [s.t as f64 * 1e-6, s.rate / 1_000.0])
        .collect();

    let y_max = v
        .rate_history
        .iter()
        .filter(|s| s.t >= t_min_us)
        .map(|s| s.rate / 1_000.0)
        .fold(1.0_f64, f64::max)
        * 1.2;

    Plot::new("event_rate")
        .height(avail.y.max(80.0_f32))
        .legend(Legend::default())
        .x_axis_label("sensor time (s)")
        .y_axis_label("kev/s")
        .auto_bounds(egui::Vec2b::new(false, false))
        .show(ui, |plot_ui| {
            plot_ui.set_plot_bounds(PlotBounds::from_min_max([t_min_s, 0.0], [t_max_s, y_max]));
            plot_ui.line(Line::new(points).name("event rate").width(1.5_f32));
        });
}
