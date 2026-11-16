//! Center canvas: Normal frame, Time Surface, or 3-D XYT — with overlays.

use crate::state::{DisplayMode, ViewerState};
use eframe::egui;

const MIN_ZOOM: f32 = 0.5;
const MAX_ZOOM: f32 = 20.0;

/// Show the center canvas (frame, time surface, or 3-D XYT).
pub fn show_canvas(ctx: &egui::Context, v: &mut ViewerState) {
    egui::CentralPanel::default().show(ctx, |ui| {
        // ── 3-D XYT mode: full-canvas software renderer ──────────────────────
        if v.display_mode == DisplayMode::Xyt3D {
            crate::ui::xyt_view::draw_xyt_3d(ui, v);
            return;
        }

        // ── Normal / TimeSurface mode ─────────────────────────────────────────
        let tex = match v.display_mode {
            DisplayMode::TimeSurface => v.ts_texture.as_ref(),
            DisplayMode::Normal => v.frame_texture.as_ref(),
            DisplayMode::Xyt3D => unreachable!(),
        };

        let avail_rect = ui.available_rect_before_wrap();

        let Some(tex) = tex else {
            ui.centered_and_justified(|ui| {
                ui.label("Waiting for first frame…");
            });
            return;
        };

        let interaction = ui.allocate_rect(avail_rect, egui::Sense::drag() | egui::Sense::hover());

        // Base rect: fitted, unzoomed image centered in the panel.
        let base_rect = fit_rect(avail_rect, v.width, v.height);

        // ── Zoom (scroll) and pan (drag), centered on the cursor if possible ────
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll != 0.0 {
            let old_zoom = v.canvas_zoom;
            let new_zoom = (old_zoom * (1.0 + scroll * 0.001)).clamp(MIN_ZOOM, MAX_ZOOM);
            if let Some(mouse) = interaction.hover_pos() {
                let center = base_rect.center() + v.canvas_pan;
                let offset = mouse - center;
                let new_center = mouse - offset * (new_zoom / old_zoom);
                v.canvas_pan = new_center - base_rect.center();
            }
            v.canvas_zoom = new_zoom;
        }
        if interaction.dragged() {
            v.canvas_pan += interaction.drag_delta();
        }
        if interaction.double_clicked() {
            v.canvas_zoom = 1.0;
            v.canvas_pan = egui::Vec2::ZERO;
        }

        // Current image rect after zoom + pan.
        let mut image_rect = egui::Rect::from_center_size(
            base_rect.center() + v.canvas_pan,
            base_rect.size() * v.canvas_zoom,
        );

        // Keep at least part of the image visible by clamping its center.
        let allowed_min = avail_rect.min - image_rect.size() * 0.5;
        let allowed_max = avail_rect.max + image_rect.size() * 0.5;
        let clamped_center = image_rect.center().clamp(allowed_min, allowed_max);
        v.canvas_pan = clamped_center - base_rect.center();
        image_rect = egui::Rect::from_center_size(clamped_center, image_rect.size());

        // Draw the texture directly so the image does not steal canvas interactions.
        let painter = ui.painter_at(avail_rect);
        painter.image(
            tex.id(),
            image_rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );

        // ── ROI overlay (cyan rectangle) ──────────────────────────────────────
        if v.roi_enabled {
            let overlay_painter = ui.painter_at(image_rect);
            let tl = sensor_to_screen(
                v.roi_left as f32,
                v.roi_top as f32,
                image_rect,
                v.width,
                v.height,
            );
            let br = sensor_to_screen(
                (v.roi_left + v.roi_width) as f32,
                (v.roi_top + v.roi_height) as f32,
                image_rect,
                v.width,
                v.height,
            );
            overlay_painter.rect_stroke(
                egui::Rect::from_two_pos(tl, br),
                0.0_f32,
                egui::Stroke::new(2.0_f32, egui::Color32::from_rgb(137, 220, 235)),
                egui::StrokeKind::Outside,
            );
        }

        // ── Hint ──────────────────────────────────────────────────────────────
        painter.text(
            egui::pos2(avail_rect.right() - 8.0, avail_rect.bottom() - 8.0),
            egui::Align2::RIGHT_BOTTOM,
            format!(
                "zoom ×{:.1}  ·  scroll to zoom  ·  drag to pan  ·  double-click to reset",
                v.canvas_zoom
            ),
            egui::FontId::proportional(10.0),
            egui::Color32::from_rgba_unmultiplied(200, 200, 200, 140),
        );
    });
}

/// Fit the sensor image into `avail` while preserving aspect ratio.
fn fit_rect(avail: egui::Rect, width: u32, height: u32) -> egui::Rect {
    let aspect = width as f32 / height.max(1) as f32;
    let avail_w = avail.width();
    let avail_h = avail.height();
    let display_size = if avail_w / avail_h > aspect {
        egui::vec2(avail_h * aspect, avail_h)
    } else {
        egui::vec2(avail_w, avail_w / aspect)
    };
    egui::Rect::from_center_size(avail.center(), display_size)
}

/// Map a sensor-space point `(sx, sy)` to screen coordinates within `canvas`.
fn sensor_to_screen(sx: f32, sy: f32, canvas: egui::Rect, w: u32, h: u32) -> egui::Pos2 {
    egui::pos2(
        canvas.left() + (sx / w as f32) * canvas.width(),
        canvas.top() + (sy / h as f32) * canvas.height(),
    )
}
