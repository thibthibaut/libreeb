//! Center canvas: Normal frame, Time Surface, or 3-D XYT — with overlays.

use crate::state::{DisplayMode, ViewerState};
use eframe::egui;

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
            DisplayMode::Normal      => v.frame_texture.as_ref(),
            DisplayMode::Xyt3D      => unreachable!(),
        };

        let Some(tex) = tex else {
            ui.centered_and_justified(|ui| {
                ui.label("Waiting for first frame…");
            });
            return;
        };

        // Compute the image rect so overlays use the exact same rect.
        let avail_rect = ui.available_rect_before_wrap();
        let aspect  = v.width as f32 / v.height as f32;
        let avail_w = avail_rect.width();
        let avail_h = avail_rect.height();
        let display_size = if avail_w / avail_h > aspect {
            egui::vec2(avail_h * aspect, avail_h)
        } else {
            egui::vec2(avail_w, avail_w / aspect)
        };
        let image_rect = egui::Rect::from_center_size(avail_rect.center(), display_size);

        ui.put(image_rect, egui::Image::new((tex.id(), display_size)));

        let painter = ui.painter_at(image_rect);

        // ── ROI overlay (cyan rectangle) ──────────────────────────────────────
        if v.roi_enabled {
            let tl = sensor_to_screen(
                v.roi_left as f32,
                v.roi_top  as f32,
                image_rect, v.width, v.height,
            );
            let br = sensor_to_screen(
                (v.roi_left + v.roi_width)  as f32,
                (v.roi_top  + v.roi_height) as f32,
                image_rect, v.width, v.height,
            );
            painter.rect_stroke(
                egui::Rect::from_two_pos(tl, br),
                0.0_f32,
                egui::Stroke::new(2.0_f32, egui::Color32::from_rgb(137, 220, 235)),
                egui::StrokeKind::Outside,
            );
        }
    });
}

/// Map a sensor-space point `(sx, sy)` to screen coordinates within `canvas`.
fn sensor_to_screen(sx: f32, sy: f32, canvas: egui::Rect, w: u32, h: u32) -> egui::Pos2 {
    egui::pos2(
        canvas.left() + (sx / w as f32) * canvas.width(),
        canvas.top()  + (sy / h as f32) * canvas.height(),
    )
}
