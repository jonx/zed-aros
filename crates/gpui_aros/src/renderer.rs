//! tiny-skia CPU renderer for the GPUI scene.
//!
//! Real handling for solid Quads (rounded corners + borders) and
//! MonochromeSprites (glyphs: atlas alpha tinted by color), both clipped to the
//! primitive's content mask. Shadows / Paths / Underlines / Polychrome /
//! Subpixel / Surfaces are TODO no-ops this milestone. Subpixel rendering is
//! reported unsupported, so GPUI never emits SubpixelSprites.

use std::sync::Arc;

use gpui::{
    Bounds, ContentMask, DevicePixels, Hsla, PrimitiveBatch, Quad, Rgba, ScaledPixels, Scene, Size,
};
use tiny_skia::{
    Color, FillRule, Mask, Paint, PathBuilder, Pixmap, PixmapPaint, PremultipliedColorU8, Rect,
    Stroke, Transform,
};

use crate::atlas::CpuAtlas;

pub(crate) struct CpuRenderer {
    pixmap: Pixmap,
    atlas: Arc<CpuAtlas>,
    clip_key: Option<ClipKey>,
    clip_mask: Option<Mask>,
}

type ClipKey = (i32, i32, i32, i32);

impl CpuRenderer {
    pub(crate) fn new(size: Size<DevicePixels>) -> Self {
        let (w, h) = clamp_size(size);
        Self {
            pixmap: Pixmap::new(w, h).expect("failed to allocate framebuffer pixmap"),
            atlas: Arc::new(CpuAtlas::new()),
            clip_key: None,
            clip_mask: None,
        }
    }

    pub(crate) fn sprite_atlas(&self) -> Arc<CpuAtlas> {
        self.atlas.clone()
    }

    pub(crate) fn update_drawable_size(&mut self, size: Size<DevicePixels>) {
        let (w, h) = clamp_size(size);
        if self.pixmap.width() != w || self.pixmap.height() != h {
            self.pixmap = Pixmap::new(w, h).expect("failed to allocate framebuffer pixmap");
            self.clip_key = None;
            self.clip_mask = None;
        }
    }

    /// Premultiplied RGBA bytes of the framebuffer (opaque frames, so
    /// premultiplied == straight; RECTFMT_RGBA in the glue matches this order).
    pub(crate) fn framebuffer(&self) -> &[u8] {
        self.pixmap.data()
    }

    pub(crate) fn width(&self) -> u32 {
        self.pixmap.width()
    }

    pub(crate) fn height(&self) -> u32 {
        self.pixmap.height()
    }

    pub(crate) fn draw(&mut self, scene: &Scene) {
        self.pixmap.fill(Color::BLACK);

        for batch in scene.batches() {
            match batch {
                PrimitiveBatch::Quads(range) => {
                    for quad in &scene.quads[range] {
                        self.draw_quad(quad);
                    }
                }
                PrimitiveBatch::MonochromeSprites { range, .. } => {
                    for sprite in &scene.monochrome_sprites[range] {
                        self.draw_monochrome_sprite(sprite);
                    }
                }
                // TODO: shadows, paths, underlines, polychrome sprites, surfaces.
                _ => {}
            }
        }
    }

    fn draw_quad(&mut self, quad: &Quad) {
        let bounds = quad.bounds;
        self.update_clip(&quad.content_mask);
        let mask = self.clip_mask.as_ref();

        // Background fill (solid only; gradients are TODO).
        if let Some(color) = quad.background.as_solid() {
            if !color.is_transparent() {
                let paint = solid_paint(color);
                if is_square(&quad.corner_radii) {
                    if let Some(rect) = to_rect(&bounds) {
                        self.pixmap.fill_rect(rect, &paint, Transform::identity(), mask);
                    }
                } else if let Some(path) = rounded_rect_path(&bounds, &quad.corner_radii) {
                    self.pixmap.fill_path(
                        &path,
                        &paint,
                        FillRule::Winding,
                        Transform::identity(),
                        mask,
                    );
                }
            }
        }

        // Border: approximate per-edge widths with a single stroke of the
        // widest edge (TODO: honor differing edge widths).
        let border_width = quad
            .border_widths
            .top
            .0
            .max(quad.border_widths.right.0)
            .max(quad.border_widths.bottom.0)
            .max(quad.border_widths.left.0);
        if border_width > 0.0 && !quad.border_color.is_transparent() {
            let mask = self.clip_mask.as_ref();
            let paint = solid_paint(quad.border_color);
            let stroke = Stroke {
                width: border_width,
                ..Stroke::default()
            };
            // Inset the stroke by half its width so it stays inside the bounds.
            let inset = border_width / 2.0;
            let inset_bounds = inset_bounds(&bounds, inset);
            if let Some(path) = rounded_rect_path(&inset_bounds, &quad.corner_radii)
                .or_else(|| to_rect(&inset_bounds).map(|r| PathBuilder::from_rect(r)))
            {
                self.pixmap
                    .stroke_path(&path, &paint, &stroke, Transform::identity(), mask);
            }
        }
    }

    fn draw_monochrome_sprite(&mut self, sprite: &gpui::MonochromeSprite) {
        let Some(tile) = self.atlas.read_tile(&sprite.tile) else {
            return;
        };
        if tile.bytes_per_pixel != 1 || tile.width == 0 || tile.height == 0 {
            return;
        }

        // Build a tinted premultiplied RGBA pixmap from the alpha coverage.
        let Some(mut sprite_pixmap) = Pixmap::new(tile.width as u32, tile.height as u32) else {
            return;
        };
        let tint: Rgba = sprite.color.into();
        let dst = sprite_pixmap.data_mut();
        for (i, &coverage) in tile.data.iter().enumerate() {
            let a = ((coverage as f32) * tint.a).round().clamp(0.0, 255.0) as u8;
            let premul = |c: f32| ((c * a as f32).round()).clamp(0.0, 255.0) as u8;
            let px = PremultipliedColorU8::from_rgba(premul(tint.r), premul(tint.g), premul(tint.b), a)
                .unwrap_or(PremultipliedColorU8::TRANSPARENT);
            let o = i * 4;
            dst[o] = px.red();
            dst[o + 1] = px.green();
            dst[o + 2] = px.blue();
            dst[o + 3] = px.alpha();
        }

        self.update_clip(&sprite.content_mask);
        let mask = self.clip_mask.as_ref();
        let x = sprite.bounds.origin.x.0.round() as i32;
        let y = sprite.bounds.origin.y.0.round() as i32;
        // TODO: honor sprite.transformation (glyphs are axis-aligned in practice).
        self.pixmap.draw_pixmap(
            x,
            y,
            sprite_pixmap.as_ref(),
            &PixmapPaint::default(),
            Transform::identity(),
            mask,
        );
    }

    /// Rebuild the clip mask for `content_mask` if it changed. A clip covering
    /// the whole framebuffer sets no mask (`clip_mask = None`).
    fn update_clip(&mut self, content_mask: &ContentMask<ScaledPixels>) {
        let b = &content_mask.bounds;
        let x0 = b.origin.x.0.floor() as i32;
        let y0 = b.origin.y.0.floor() as i32;
        let x1 = (b.origin.x.0 + b.size.width.0).ceil() as i32;
        let y1 = (b.origin.y.0 + b.size.height.0).ceil() as i32;
        let key = (x0, y0, x1, y1);

        let w = self.pixmap.width() as i32;
        let h = self.pixmap.height() as i32;
        if x0 <= 0 && y0 <= 0 && x1 >= w && y1 >= h {
            self.clip_key = None;
            self.clip_mask = None;
            return;
        }
        if self.clip_key == Some(key) {
            return;
        }

        let mut mask = Mask::new(self.pixmap.width(), self.pixmap.height());
        if let (Some(mask), Some(rect)) = (
            mask.as_mut(),
            Rect::from_xywh(
                x0 as f32,
                y0 as f32,
                (x1 - x0).max(0) as f32,
                (y1 - y0).max(0) as f32,
            ),
        ) {
            mask.fill_path(
                &PathBuilder::from_rect(rect),
                FillRule::Winding,
                false,
                Transform::identity(),
            );
        }
        self.clip_mask = mask;
        self.clip_key = Some(key);
    }
}

fn clamp_size(size: Size<DevicePixels>) -> (u32, u32) {
    let w = size.width.0.max(1) as u32;
    let h = size.height.0.max(1) as u32;
    (w, h)
}

fn to_rect(bounds: &Bounds<ScaledPixels>) -> Option<Rect> {
    Rect::from_xywh(
        bounds.origin.x.0,
        bounds.origin.y.0,
        bounds.size.width.0,
        bounds.size.height.0,
    )
}

fn inset_bounds(bounds: &Bounds<ScaledPixels>, inset: f32) -> Bounds<ScaledPixels> {
    Bounds {
        origin: gpui::Point {
            x: ScaledPixels(bounds.origin.x.0 + inset),
            y: ScaledPixels(bounds.origin.y.0 + inset),
        },
        size: Size {
            width: ScaledPixels((bounds.size.width.0 - 2.0 * inset).max(0.0)),
            height: ScaledPixels((bounds.size.height.0 - 2.0 * inset).max(0.0)),
        },
    }
}

fn is_square(radii: &gpui::Corners<ScaledPixels>) -> bool {
    radii.top_left.0 <= 0.0
        && radii.top_right.0 <= 0.0
        && radii.bottom_right.0 <= 0.0
        && radii.bottom_left.0 <= 0.0
}

/// A rounded-rect path with per-corner radii (quadratic corners), clamped so
/// radii never exceed half the shorter side.
fn rounded_rect_path(
    bounds: &Bounds<ScaledPixels>,
    radii: &gpui::Corners<ScaledPixels>,
) -> Option<tiny_skia::Path> {
    let x = bounds.origin.x.0;
    let y = bounds.origin.y.0;
    let w = bounds.size.width.0;
    let h = bounds.size.height.0;
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    let limit = (w.min(h)) / 2.0;
    let tl = radii.top_left.0.clamp(0.0, limit);
    let tr = radii.top_right.0.clamp(0.0, limit);
    let br = radii.bottom_right.0.clamp(0.0, limit);
    let bl = radii.bottom_left.0.clamp(0.0, limit);

    let mut pb = PathBuilder::new();
    pb.move_to(x + tl, y);
    pb.line_to(x + w - tr, y);
    if tr > 0.0 {
        pb.quad_to(x + w, y, x + w, y + tr);
    }
    pb.line_to(x + w, y + h - br);
    if br > 0.0 {
        pb.quad_to(x + w, y + h, x + w - br, y + h);
    }
    pb.line_to(x + bl, y + h);
    if bl > 0.0 {
        pb.quad_to(x, y + h, x, y + h - bl);
    }
    pb.line_to(x, y + tl);
    if tl > 0.0 {
        pb.quad_to(x, y, x + tl, y);
    }
    pb.close();
    pb.finish()
}

fn solid_paint<'a>(color: Hsla) -> Paint<'a> {
    let rgba: Rgba = color.into();
    let mut paint = Paint::default();
    paint.set_color(
        Color::from_rgba(
            rgba.r.clamp(0.0, 1.0),
            rgba.g.clamp(0.0, 1.0),
            rgba.b.clamp(0.0, 1.0),
            rgba.a.clamp(0.0, 1.0),
        )
        .unwrap_or(Color::TRANSPARENT),
    );
    paint.anti_alias = true;
    paint
}
