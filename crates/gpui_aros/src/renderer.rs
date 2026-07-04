//! tiny-skia CPU renderer for the GPUI scene.
//!
//! Real handling for Quads (rounded corners + borders, solid or linear-
//! gradient fills), MonochromeSprites (glyphs: atlas alpha tinted by color),
//! PolychromeSprites (images: premultiplied BGRA atlas tiles, with
//! opacity/grayscale and rounded-corner clipping), Paths (pre-tessellated
//! triangles, solid or gradient), Underlines (straight + wavy), and drop
//! Shadows (layered-ring blur approximation) — all clipped to the
//! primitive's content mask. Remaining TODO no-ops: pattern fills
//! (slash/checkerboard), inset shadows, and Surfaces (mac-only video frames,
//! never emitted here). Subpixel rendering is reported unsupported, so GPUI
//! never emits SubpixelSprites.

// Off-AROS these are only reached from the conformance tests — the
// window/platform shell that drives them in production is cfg-gated.
#![cfg_attr(not(target_os = "aros"), allow(dead_code))]

use std::sync::Arc;

use gpui::{
    Bounds, ContentMask, DevicePixels, Hsla, PrimitiveBatch, Quad, Rgba, ScaledPixels, Scene, Size,
};
use tiny_skia::{
    Color, FillRule, FilterQuality, Mask, Paint, PathBuilder, Pixmap, PixmapPaint,
    PremultipliedColorU8, Rect, Stroke, Transform,
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

    // -- Test hooks (porting conformance suite; see conformance.rs) ------
    #[cfg(test)]
    pub(crate) fn sprite_atlas_concrete(&self) -> Arc<CpuAtlas> {
        self.atlas.clone()
    }

    #[cfg(test)]
    pub(crate) fn draw_monochrome_sprite_for_test(&mut self, s: &gpui::MonochromeSprite) {
        self.draw_monochrome_sprite(s);
    }

    #[cfg(test)]
    pub(crate) fn draw_quad_for_test(&mut self, q: &Quad) {
        self.draw_quad(q);
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
                PrimitiveBatch::Shadows(range) => {
                    for shadow in &scene.shadows[range] {
                        self.draw_shadow(shadow);
                    }
                }
                PrimitiveBatch::Quads(range) => {
                    for quad in &scene.quads[range] {
                        self.draw_quad(quad);
                    }
                }
                PrimitiveBatch::Paths(range) => {
                    for path in &scene.paths[range] {
                        self.draw_path(path);
                    }
                }
                PrimitiveBatch::Underlines(range) => {
                    for underline in &scene.underlines[range] {
                        self.draw_underline(underline);
                    }
                }
                PrimitiveBatch::MonochromeSprites { range, .. } => {
                    for sprite in &scene.monochrome_sprites[range] {
                        self.draw_monochrome_sprite(sprite);
                    }
                }
                PrimitiveBatch::PolychromeSprites { range, .. } => {
                    for sprite in &scene.polychrome_sprites[range] {
                        self.draw_polychrome_sprite(sprite);
                    }
                }
                // Surfaces are mac-only video frames — never emitted here.
                _ => {}
            }
        }
    }

    fn draw_quad(&mut self, quad: &Quad) {
        let bounds = quad.bounds;
        self.update_clip(&quad.content_mask);
        let mask = self.clip_mask.as_ref();

        // Background fill: solid or two-stop linear gradient. Patterns
        // (slash/checkerboard) have no accessor yet and stay TODO.
        if let Some(paint) = background_paint(&quad.background, &bounds) {
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
        // The GPU backends *sample* the tile into the layout bounds; blit
        // 1:1 only when they match (glyphs — keeps text pixel-crisp) and
        // scale otherwise. SVGs are rasterized at 2× the layout size
        // (gpui's SMOOTH_SVG_SCALE_FACTOR) precisely so this downsample
        // smooths them — drawing them unscaled painted icons at double
        // size, clipped by the content mask.
        let dst_w = sprite.bounds.size.width.0;
        let dst_h = sprite.bounds.size.height.0;
        if dst_w <= 0.0 || dst_h <= 0.0 {
            return;
        }
        let sx = dst_w / tile.width as f32;
        let sy = dst_h / tile.height as f32;
        let x = sprite.bounds.origin.x.0;
        let y = sprite.bounds.origin.y.0;
        // TODO: honor sprite.transformation (glyphs are axis-aligned in practice).
        if (sx - 1.0).abs() < 0.001 && (sy - 1.0).abs() < 0.001 {
            self.pixmap.draw_pixmap(
                x.round() as i32,
                y.round() as i32,
                sprite_pixmap.as_ref(),
                &PixmapPaint::default(),
                Transform::identity(),
                mask,
            );
        } else {
            let paint = PixmapPaint {
                quality: FilterQuality::Bilinear,
                ..PixmapPaint::default()
            };
            self.pixmap.draw_pixmap(
                0,
                0,
                sprite_pixmap.as_ref(),
                &paint,
                Transform::from_scale(sx, sy).post_translate(x, y),
                mask,
            );
        }
    }

    /// Images: premultiplied BGRA atlas tiles blitted (scaled when the layout
    /// size differs from the tile), honoring opacity, the grayscale flag, and
    /// rounded corners (via a one-off mask — corner clipping composes with the
    /// rectangular content mask).
    fn draw_polychrome_sprite(&mut self, sprite: &gpui::PolychromeSprite) {
        let Some(tile) = self.atlas.read_tile(&sprite.tile) else {
            return;
        };
        if tile.bytes_per_pixel != 4 || tile.width == 0 || tile.height == 0 {
            return;
        }

        let Some(mut sprite_pixmap) = Pixmap::new(tile.width as u32, tile.height as u32) else {
            return;
        };
        // Atlas polychrome bytes are premultiplied BGRA (gpui's image
        // convention); tiny-skia wants premultiplied RGBA — swap B/R,
        // scaling by opacity (and collapsing to luma when grayscale).
        let opacity = sprite.opacity.clamp(0.0, 1.0);
        let dst = sprite_pixmap.data_mut();
        for (src, out) in tile.data.chunks_exact(4).zip(dst.chunks_exact_mut(4)) {
            let (b, g, r, a) = (src[0] as f32, src[1] as f32, src[2] as f32, src[3] as f32);
            let (r, g, b) = if sprite.grayscale {
                // Premultiplied channels share the pixel's alpha, so the
                // luma weights apply directly.
                let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
                (y, y, y)
            } else {
                (r, g, b)
            };
            out[0] = (r * opacity).round().clamp(0.0, 255.0) as u8;
            out[1] = (g * opacity).round().clamp(0.0, 255.0) as u8;
            out[2] = (b * opacity).round().clamp(0.0, 255.0) as u8;
            out[3] = (a * opacity).round().clamp(0.0, 255.0) as u8;
        }

        self.update_clip(&sprite.content_mask);
        // Rounded corners need a shape clip on top of the rectangular
        // content mask; build a one-off intersection so the cached
        // rect-only mask stays valid for the next primitive.
        let corner_mask = if is_square(&sprite.corner_radii) {
            None
        } else {
            rounded_rect_path(&sprite.bounds, &sprite.corner_radii).map(|path| {
                let mut mask = match self.clip_mask.clone() {
                    Some(m) => m,
                    None => {
                        let mut m = Mask::new(self.pixmap.width(), self.pixmap.height())
                            .expect("mask allocation follows the framebuffer, which allocated");
                        m.fill_path(
                            &PathBuilder::from_rect(
                                Rect::from_xywh(
                                    0.0,
                                    0.0,
                                    self.pixmap.width() as f32,
                                    self.pixmap.height() as f32,
                                )
                                .expect("framebuffer dims are positive"),
                            ),
                            FillRule::Winding,
                            true,
                            Transform::identity(),
                        );
                        m
                    }
                };
                mask.intersect_path(&path, FillRule::Winding, true, Transform::identity());
                mask
            })
        };
        let mask = corner_mask.as_ref().or(self.clip_mask.as_ref());

        // Blit, scaling when layout and tile sizes differ (fractional
        // display scales, zoomed images).
        let dst_w = sprite.bounds.size.width.0;
        let dst_h = sprite.bounds.size.height.0;
        if dst_w <= 0.0 || dst_h <= 0.0 {
            return;
        }
        let sx = dst_w / tile.width as f32;
        let sy = dst_h / tile.height as f32;
        let x = sprite.bounds.origin.x.0;
        let y = sprite.bounds.origin.y.0;
        if (sx - 1.0).abs() < 0.001 && (sy - 1.0).abs() < 0.001 {
            self.pixmap.draw_pixmap(
                x.round() as i32,
                y.round() as i32,
                sprite_pixmap.as_ref(),
                &PixmapPaint::default(),
                Transform::identity(),
                mask,
            );
        } else {
            let paint = PixmapPaint {
                quality: FilterQuality::Bilinear,
                ..PixmapPaint::default()
            };
            self.pixmap.draw_pixmap(
                0,
                0,
                sprite_pixmap.as_ref(),
                &paint,
                Transform::from_scale(sx, sy).post_translate(x, y),
                mask,
            );
        }
    }

    /// Pre-tessellated vector paths (tab curves, chart lines, …): every
    /// triangle appended into one tiny-skia path filled with the nonzero
    /// winding rule, so shared edges can't double-blend at partial alpha.
    /// Anti-aliasing stays off for the same reason — edge AA would show
    /// seams between adjacent triangles instead of a smooth silhouette.
    fn draw_path(&mut self, path: &gpui::Path<ScaledPixels>) {
        if path.vertices.is_empty() {
            return;
        }
        let Some(mut paint) = background_paint(&path.color, &path.bounds) else {
            return;
        };
        let mut pb = PathBuilder::new();
        for tri in path.vertices.chunks_exact(3) {
            pb.move_to(tri[0].xy_position.x.0, tri[0].xy_position.y.0);
            pb.line_to(tri[1].xy_position.x.0, tri[1].xy_position.y.0);
            pb.line_to(tri[2].xy_position.x.0, tri[2].xy_position.y.0);
            pb.close();
        }
        let Some(skia_path) = pb.finish() else {
            return;
        };
        self.update_clip(&path.content_mask);
        let mask = self.clip_mask.as_ref();
        paint.anti_alias = false;
        self.pixmap
            .fill_path(&skia_path, &paint, FillRule::Winding, Transform::identity(), mask);
    }

    /// Underlines. Straight ones fill their bounds (already sized to the
    /// line thickness by the paint layer); wavy ones (diagnostics squiggles)
    /// stroke a sine wave across the bounds — cubic segments per half-wave,
    /// amplitude from the bounds height, ~6×-thickness wavelength, visually
    /// matching the GPU shaders without reproducing their exact math.
    fn draw_underline(&mut self, underline: &gpui::Underline) {
        if underline.color.is_transparent() {
            return;
        }
        self.update_clip(&underline.content_mask);
        let mask = self.clip_mask.as_ref();
        let paint = solid_paint(underline.color);
        let b = &underline.bounds;
        let thickness = underline.thickness.0.max(0.5);

        if underline.wavy == 0 {
            if let Some(rect) = Rect::from_xywh(
                b.origin.x.0,
                b.origin.y.0,
                b.size.width.0,
                b.size.height.0.min(thickness).max(0.5),
            ) {
                self.pixmap
                    .fill_rect(rect, &paint, Transform::identity(), mask);
            }
            return;
        }

        let width = b.size.width.0;
        if width <= 0.0 {
            return;
        }
        let amplitude = ((b.size.height.0 - thickness) / 2.0).max(0.5);
        let mid_y = b.origin.y.0 + b.size.height.0 / 2.0;
        let half_wave = (3.0 * thickness).max(2.0);
        // K: the classic cubic-Bezier sine approximation constant for a
        // half-period (control points at 1/3 and 2/3 of the span).
        const K: f32 = 0.3642;
        let mut pb = PathBuilder::new();
        pb.move_to(b.origin.x.0, mid_y);
        let mut x = b.origin.x.0;
        let mut sign = -1.0f32; // first crest points up (screen-y down)
        while x < b.origin.x.0 + width {
            let x_end = (x + half_wave).min(b.origin.x.0 + width);
            let span = x_end - x;
            let peak = sign * amplitude * (span / half_wave);
            pb.cubic_to(
                x + span * K,
                mid_y + peak,
                x_end - span * K,
                mid_y + peak,
                x_end,
                mid_y,
            );
            x = x_end;
            sign = -sign;
        }
        if let Some(wave) = pb.finish() {
            let stroke = Stroke {
                width: thickness,
                ..Stroke::default()
            };
            self.pixmap
                .stroke_path(&wave, &paint, &stroke, Transform::identity(), mask);
        }
    }

    /// Drop shadows, approximated: tiny-skia has no gaussian blur, so we
    /// layer a handful of concentric rounded rects from the outer (blurred)
    /// extent inward, each adding a slice of the shadow's alpha — a cheap
    /// falloff that reads as soft at UI shadow sizes. Inset shadows are
    /// rare in GPUI chrome and stay TODO.
    fn draw_shadow(&mut self, shadow: &gpui::Shadow) {
        if shadow.inset != 0 || shadow.color.is_transparent() {
            return;
        }
        self.update_clip(&shadow.content_mask);

        let blur = shadow.blur_radius.0.max(0.0);
        let base: Rgba = shadow.color.into();
        if blur < 1.0 {
            // Sharp shadow: a single fill of the quad.
            if let Some(path) = rounded_rect_path(&shadow.bounds, &shadow.corner_radii) {
                let mask = self.clip_mask.as_ref();
                self.pixmap.fill_path(
                    &path,
                    &solid_paint(shadow.color),
                    FillRule::Winding,
                    Transform::identity(),
                    mask,
                );
            }
            return;
        }

        // Cumulative alpha per layer approximating a smooth falloff:
        // innermost layer is covered by all fills (≈ full alpha), the
        // outer band only by the faintest one.
        const WEIGHTS: [f32; 4] = [0.10, 0.15, 0.25, 0.50];
        for (i, w) in WEIGHTS.iter().enumerate() {
            let inset = blur * (1.0 - (i as f32 + 1.0) / WEIGHTS.len() as f32);
            let bounds = inset_bounds(&shadow.bounds, inset);
            let radii = inset_radii(&shadow.corner_radii, inset);
            let Some(path) = rounded_rect_path(&bounds, &radii)
                .or_else(|| to_rect(&bounds).map(PathBuilder::from_rect))
            else {
                continue;
            };
            let mut layer = base;
            layer.a *= w;
            let mask = self.clip_mask.as_ref();
            self.pixmap.fill_path(
                &path,
                &solid_paint(Hsla::from(layer)),
                FillRule::Winding,
                Transform::identity(),
                mask,
            );
        }
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

/// Corner radii shrunk to follow bounds inset by `inset` (never negative).
fn inset_radii(radii: &gpui::Corners<ScaledPixels>, inset: f32) -> gpui::Corners<ScaledPixels> {
    let shrink = |r: ScaledPixels| ScaledPixels((r.0 - inset).max(0.0));
    gpui::Corners {
        top_left: shrink(radii.top_left),
        top_right: shrink(radii.top_right),
        bottom_right: shrink(radii.bottom_right),
        bottom_left: shrink(radii.bottom_left),
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

/// A tiny-skia paint for a gpui `Background`: solid colors and two-stop
/// linear gradients (interpolated in sRGB — the Oklab color-space option is
/// approximated by sRGB, acceptable for the subtle UI fades gpui uses).
/// `None` for fully transparent fills and the pattern tags (slash /
/// checkerboard: no public accessor yet, and unused by app chrome).
fn background_paint<'a>(
    background: &gpui::Background,
    bounds: &Bounds<ScaledPixels>,
) -> Option<Paint<'a>> {
    if let Some(color) = background.as_solid() {
        if color.is_transparent() {
            return None;
        }
        return Some(solid_paint(color));
    }
    let (angle_deg, stops) = background.as_linear_gradient()?;

    // CSS convention: 0deg points up, angles run clockwise. The gradient
    // line passes through the box center; its half-length is the projection
    // of the half-extent onto the direction (the CSS gradient-line length),
    // so the first/last stops land exactly on the box corners' shadow.
    let theta = angle_deg.to_radians();
    let (dx, dy) = (theta.sin(), -theta.cos());
    let w = bounds.size.width.0;
    let h = bounds.size.height.0;
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    let cx = bounds.origin.x.0 + w / 2.0;
    let cy = bounds.origin.y.0 + h / 2.0;
    let half_len = (w * dx.abs() + h * dy.abs()) / 2.0;

    let to_color = |hsla: Hsla| {
        let rgba: Rgba = hsla.into();
        Color::from_rgba(
            rgba.r.clamp(0.0, 1.0),
            rgba.g.clamp(0.0, 1.0),
            rgba.b.clamp(0.0, 1.0),
            rgba.a.clamp(0.0, 1.0),
        )
        .unwrap_or(Color::TRANSPARENT)
    };
    let shader = tiny_skia::LinearGradient::new(
        tiny_skia::Point::from_xy(cx - dx * half_len, cy - dy * half_len),
        tiny_skia::Point::from_xy(cx + dx * half_len, cy + dy * half_len),
        vec![
            tiny_skia::GradientStop::new(
                stops[0].percentage.clamp(0.0, 1.0),
                to_color(stops[0].color),
            ),
            tiny_skia::GradientStop::new(
                stops[1].percentage.clamp(0.0, 1.0),
                to_color(stops[1].color),
            ),
        ],
        tiny_skia::SpreadMode::Pad,
        Transform::identity(),
    )?;
    let mut paint = Paint::default();
    paint.shader = shader;
    paint.anti_alias = true;
    Some(paint)
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
