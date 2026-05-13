//! 3-D XYT software-rendered point cloud.
//!
//! Sensor coordinate system → 3-D space:
//!   X  → sensor columns  (0 = left, increases right)
//!   Y  → sensor rows     (0 = top,  flipped so +Y is up in 3-D)
//!   T  → time            (oldest = back/−0.5, newest = front/+0.5)
//!
//! Camera orbits origin via azimuth (around Y) then elevation (around X).
//!
//! View presets (set from info panel):
//!   3D  → az=45°, el=30°
//!   XY  → az=0°,  el=90°
//!   XT  → az=0°,  el=0°
//!   YT  → az=90°, el=0°

use crate::state::ViewerState;
use eframe::egui;

/// Render the 3-D XYT point cloud into the full available area of `ui`.
/// Called from `canvas.rs` when `DisplayMode::Xyt3D` is active.
pub fn draw_xyt_3d(ui: &mut egui::Ui, v: &mut ViewerState) {
    let avail = ui.available_size();
    let canvas_size = avail.min_elem().max(80.0);
    let (canvas_response, painter) =
        ui.allocate_painter(avail, egui::Sense::drag() | egui::Sense::hover());

    // ── Mouse drag → rotate ───────────────────────────────────────────────────
    if canvas_response.dragged() {
        let delta = canvas_response.drag_delta();
        v.xyt_azimuth   += delta.x * 0.5;
        v.xyt_elevation  = (v.xyt_elevation - delta.y * 0.5).clamp(-89.0, 89.0);
    }

    // ── Scroll wheel → zoom ───────────────────────────────────────────────────
    let scroll = ui.input(|i| i.smooth_scroll_delta.y);
    if scroll != 0.0 {
        v.xyt_zoom = (v.xyt_zoom * (1.0 + scroll * 0.002)).clamp(0.1, 20.0);
    }

    let rect  = canvas_response.rect;
    let cx    = rect.center().x;
    let cy    = rect.center().y;
    let scale = canvas_size * 0.38 * v.xyt_zoom;

    // ── Background ────────────────────────────────────────────────────────────
    painter.rect_filled(rect, 0.0_f32, egui::Color32::from_rgb(15, 15, 25));

    // ── Precompute trig ───────────────────────────────────────────────────────
    let az = v.xyt_azimuth.to_radians();
    let el = v.xyt_elevation.to_radians();
    let (sa, ca) = (az.sin(), az.cos());
    let (se, ce) = (el.sin(), el.cos());

    // ── Axes ──────────────────────────────────────────────────────────────────
    draw_axes(&painter, cx, cy, scale, sa, ca, se, ce);

    // ── T-axis normalisation ──────────────────────────────────────────────────
    // newest = +0.5 (front of cloud), oldest = −0.5 (back).
    // Uses sensor timestamps from the buffer; the TimeWindow's clock-reset
    // guard ensures these are always within a coherent 2-second window.
    let newest_t = v.xyt_buffer.newest_t().unwrap_or(0);
    let oldest_t = v.xyt_buffer.oldest_t().unwrap_or(newest_t.saturating_sub(2_000_000));
    let t_range  = (newest_t - oldest_t).max(1) as f32;

    let inv_w   = 1.0 / v.width  as f32;
    let inv_h   = 1.0 / v.height as f32;
    let pt_size = v.xyt_point_size;

    // ── Collect and project events ────────────────────────────────────────────
    // Per-batch subsampling: each batch gets an equal share of the display
    // budget.  Sparse batches (few events) use step=1 and show every event;
    // dense batches are subsampled so the total stays near MAX_DISPLAY_POINTS.
    // This gives visually balanced temporal density across the cloud.
    const MAX_DISPLAY_POINTS: usize = 30_000;
    let num_batches      = v.xyt_buffer.len().max(1);
    let budget_per_batch = (MAX_DISPLAY_POINTS / num_batches).max(1);

    for batch in v.xyt_buffer.iter() {
        let step = (batch.len() / budget_per_batch).max(1);
        for i in (0..batch.len()).step_by(step) {
            // Normalise to [−0.5, +0.5].
            // Y is flipped: sensor row 0 (top) → +0.5 in 3-D (up).
            let nx =  (batch.x[i] as f32 * inv_w) - 0.5;
            let ny =  0.5 - (batch.y[i] as f32 * inv_h);
            let nt =  ((batch.t[i] - oldest_t) as f32 / t_range) - 0.5;

            // Rotate: azimuth around Y, then elevation around X.
            let rx =  nx * ca + nt * sa;
            let ry =  ny;
            let rz = -nx * sa + nt * ca;

            let fx =  rx;
            let fy =  ry * ce - rz * se;
            let fz =  ry * se + rz * ce;

            // Perspective (camera at +Z = 2.0).
            let d  = 2.0_f32;
            let sx = cx + fx / (fz + d) * scale;
            let sy = cy - fy / (fz + d) * scale;

            let color = if batch.p[i] != 0 {
                egui::Color32::from_rgb(245, 194, 231) // ON  — pink
            } else {
                egui::Color32::from_rgb(137, 220, 235) // OFF — cyan
            };

            painter.rect_filled(
                egui::Rect::from_center_size(
                    egui::pos2(sx, sy),
                    egui::vec2(pt_size, pt_size),
                ),
                0.0_f32,
                color,
            );
        }
    }

    // ── Axis labels ───────────────────────────────────────────────────────────
    let axis_labels = [
        ("X", project( 0.5,  0.0, 0.0, sa, ca, se, ce, cx, cy, scale)),
        ("Y", project( 0.0,  0.5, 0.0, sa, ca, se, ce, cx, cy, scale)),
        ("T", project( 0.0,  0.0, 0.5, sa, ca, se, ce, cx, cy, scale)),
    ];
    for (name, (px, py)) in axis_labels {
        let dir_x = px - cx;
        let dir_y = py - cy;
        let len   = (dir_x * dir_x + dir_y * dir_y).sqrt().max(0.01);
        painter.text(
            egui::pos2(px + dir_x / len * 8.0, py + dir_y / len * 8.0),
            egui::Align2::CENTER_CENTER,
            name,
            egui::FontId::proportional(12.0),
            egui::Color32::WHITE,
        );
    }

    // ── Hint ─────────────────────────────────────────────────────────────────
    painter.text(
        egui::pos2(rect.right() - 6.0, rect.bottom() - 6.0),
        egui::Align2::RIGHT_BOTTOM,
        format!("zoom ×{:.1}  scroll to zoom · drag to rotate", v.xyt_zoom),
        egui::FontId::proportional(10.0),
        egui::Color32::from_rgba_unmultiplied(200, 200, 200, 120),
    );
}

// ── Helpers ───────────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn project(
    nx: f32, ny: f32, nt: f32,
    sa: f32, ca: f32, se: f32, ce: f32,
    cx: f32, cy: f32, scale: f32,
) -> (f32, f32) {
    let rx =  nx * ca + nt * sa;
    let ry =  ny;
    let rz = -nx * sa + nt * ca;
    let fx =  rx;
    let fy =  ry * ce - rz * se;
    let fz =  ry * se + rz * ce;
    let d  = 2.0_f32;
    (cx + fx / (fz + d) * scale, cy - fy / (fz + d) * scale)
}

fn draw_axes(
    painter: &egui::Painter,
    cx: f32, cy: f32, scale: f32,
    sa: f32, ca: f32, se: f32, ce: f32,
) {
    let origin = project(0.0, 0.0, 0.0, sa, ca, se, ce, cx, cy, scale);
    let x_end  = project(0.5, 0.0, 0.0, sa, ca, se, ce, cx, cy, scale);
    let y_end  = project(0.0, 0.5, 0.0, sa, ca, se, ce, cx, cy, scale);
    let t_end  = project(0.0, 0.0, 0.5, sa, ca, se, ce, cx, cy, scale);

    let stroke = |c| egui::Stroke::new(1.5_f32, c);
    painter.line_segment(
        [egui::pos2(origin.0, origin.1), egui::pos2(x_end.0, x_end.1)],
        stroke(egui::Color32::from_rgb(255, 80, 80)),
    );
    painter.line_segment(
        [egui::pos2(origin.0, origin.1), egui::pos2(y_end.0, y_end.1)],
        stroke(egui::Color32::from_rgb(80, 255, 80)),
    );
    painter.line_segment(
        [egui::pos2(origin.0, origin.1), egui::pos2(t_end.0, t_end.1)],
        stroke(egui::Color32::from_rgb(80, 160, 255)),
    );
}
