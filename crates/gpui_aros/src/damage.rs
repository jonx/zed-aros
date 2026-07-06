//! Dirty-rect damage tracking for the CPU renderer.
//!
//! GPUI assumes a GPU and repaints every frame; rasterizing the full window
//! on the CPU each time is where "feels native" dies. Instead the renderer
//! captures a compact fingerprint of every primitive in paint order, diffs
//! it against the previous frame (common prefix + common suffix, the classic
//! display-list diff), and re-rasterizes/blits only the union of the
//! changed primitives' rectangles. Painter's algorithm stays correct
//! because the whole scene is re-walked with the damage rect as a clip:
//! unchanged primitives overlapping the damage repaint too, in order.
//!
//! Pure Rust, no OS dependency — unit-tested on the host (see the tests
//! below and `conformance.rs`).

use gpui::{
    Background, Bounds, ContentMask, Hsla, MonochromeSprite, Path, PolychromeSprite, Quad,
    ScaledPixels, Scene, Shadow, Underline,
};

/// Integer device-pixel rect, closed-open ([x0, x1) x [y0, y1)).
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub(crate) struct DamageRect {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

impl DamageRect {
    pub const EMPTY: Self = Self {
        x0: i32::MAX,
        y0: i32::MAX,
        x1: i32::MIN,
        y1: i32::MIN,
    };

    pub fn is_empty(&self) -> bool {
        self.x0 >= self.x1 || self.y0 >= self.y1
    }

    pub fn union(&mut self, other: DamageRect) {
        self.x0 = self.x0.min(other.x0);
        self.y0 = self.y0.min(other.y0);
        self.x1 = self.x1.max(other.x1);
        self.y1 = self.y1.max(other.y1);
    }

    pub fn intersect(&self, other: DamageRect) -> DamageRect {
        DamageRect {
            x0: self.x0.max(other.x0),
            y0: self.y0.max(other.y0),
            x1: self.x1.min(other.x1),
            y1: self.y1.min(other.y1),
        }
    }

    pub fn intersects(&self, other: &DamageRect) -> bool {
        !self.intersect(*other).is_empty()
    }
}

/// One primitive's identity for frame diffing: its padded paint rectangle
/// plus a content fingerprint. Two fingerprints comparing equal means the
/// primitive would rasterize identically in both frames.
#[derive(PartialEq, Clone, Debug)]
pub(crate) enum Fingerprint {
    Quad(QuadFp),
    Shadow(ShadowFp),
    Underline(UnderlineFp),
    Mono(MonoFp),
    Poly(PolyFp),
    Path(PathFp),
}

impl Fingerprint {
    pub fn rect(&self) -> DamageRect {
        match self {
            Fingerprint::Quad(f) => f.rect,
            Fingerprint::Shadow(f) => f.rect,
            Fingerprint::Underline(f) => f.rect,
            Fingerprint::Mono(f) => f.rect,
            Fingerprint::Poly(f) => f.rect,
            Fingerprint::Path(f) => f.rect,
        }
    }
}

// Hsla doesn't derive PartialEq; bit-compare the channels (colors come from
// the same computations frame to frame, so bit equality is the right test).
fn hsla_eq(a: &Hsla, b: &Hsla) -> bool {
    a.h.to_bits() == b.h.to_bits()
        && a.s.to_bits() == b.s.to_bits()
        && a.l.to_bits() == b.l.to_bits()
        && a.a.to_bits() == b.a.to_bits()
}

/// Wrapper giving [`Hsla`] the bitwise equality above.
#[derive(Copy, Clone, Debug)]
pub(crate) struct HslaBits(Hsla);

impl PartialEq for HslaBits {
    fn eq(&self, other: &Self) -> bool {
        hsla_eq(&self.0, &other.0)
    }
}

#[derive(PartialEq, Clone, Debug)]
pub(crate) struct QuadFp {
    rect: DamageRect,
    bounds: Bounds<ScaledPixels>,
    mask: Bounds<ScaledPixels>,
    background: Background,
    border_color: HslaBits,
    corner_radii: [u32; 4],
    border_widths: [u32; 4],
    style: u8,
}

#[derive(PartialEq, Clone, Debug)]
pub(crate) struct ShadowFp {
    rect: DamageRect,
    bounds: Bounds<ScaledPixels>,
    mask: Bounds<ScaledPixels>,
    color: HslaBits,
    blur: u32,
    corner_radii: [u32; 4],
}

#[derive(PartialEq, Clone, Debug)]
pub(crate) struct UnderlineFp {
    rect: DamageRect,
    bounds: Bounds<ScaledPixels>,
    mask: Bounds<ScaledPixels>,
    color: HslaBits,
    thickness: u32,
    wavy: u32,
}

#[derive(PartialEq, Clone, Debug)]
pub(crate) struct MonoFp {
    rect: DamageRect,
    bounds: Bounds<ScaledPixels>,
    mask: Bounds<ScaledPixels>,
    color: HslaBits,
    tile: gpui::AtlasTile,
    transformation: gpui::TransformationMatrix,
}

#[derive(PartialEq, Clone, Debug)]
pub(crate) struct PolyFp {
    rect: DamageRect,
    bounds: Bounds<ScaledPixels>,
    mask: Bounds<ScaledPixels>,
    tile: gpui::AtlasTile,
    corner_radii: [u32; 4],
    opacity: u32,
    grayscale: bool,
}

#[derive(PartialEq, Clone, Debug)]
pub(crate) struct PathFp {
    rect: DamageRect,
    bounds: Bounds<ScaledPixels>,
    mask: Bounds<ScaledPixels>,
    color: Background,
    /// FNV-1a over the vertex coordinates: paths carry their geometry in a
    /// Vec, so the fingerprint digests it instead of copying it.
    vertices: u64,
    vertex_count: usize,
}

fn px_bits(px: ScaledPixels) -> u32 {
    px.0.to_bits()
}

fn corners_bits(c: &gpui::Corners<ScaledPixels>) -> [u32; 4] {
    [
        px_bits(c.top_left),
        px_bits(c.top_right),
        px_bits(c.bottom_left),
        px_bits(c.bottom_right),
    ]
}

fn edges_bits(e: &gpui::Edges<ScaledPixels>) -> [u32; 4] {
    [
        px_bits(e.top),
        px_bits(e.right),
        px_bits(e.bottom),
        px_bits(e.left),
    ]
}

/// Padded integer paint rect: bounds ∩ content mask, rounded out, plus
/// `pad` pixels for anti-aliasing bleed.
fn paint_rect(
    bounds: &Bounds<ScaledPixels>,
    mask: &ContentMask<ScaledPixels>,
    pad: i32,
) -> DamageRect {
    let bx0 = bounds.origin.x.0;
    let by0 = bounds.origin.y.0;
    let bx1 = bx0 + bounds.size.width.0;
    let by1 = by0 + bounds.size.height.0;
    let mx0 = mask.bounds.origin.x.0;
    let my0 = mask.bounds.origin.y.0;
    let mx1 = mx0 + mask.bounds.size.width.0;
    let my1 = my0 + mask.bounds.size.height.0;
    DamageRect {
        x0: (bx0.max(mx0).floor() as i32) - pad,
        y0: (by0.max(my0).floor() as i32) - pad,
        x1: (bx1.min(mx1).ceil() as i32) + pad,
        y1: (by1.min(my1).ceil() as i32) + pad,
    }
}

const AA_PAD: i32 = 2;

pub(crate) fn fingerprint_quad(q: &Quad) -> Fingerprint {
    Fingerprint::Quad(QuadFp {
        rect: paint_rect(&q.bounds, &q.content_mask, AA_PAD),
        bounds: q.bounds,
        mask: q.content_mask.bounds,
        background: q.background,
        border_color: HslaBits(q.border_color),
        corner_radii: corners_bits(&q.corner_radii),
        border_widths: edges_bits(&q.border_widths),
        style: q.border_style as u8,
    })
}

pub(crate) fn fingerprint_shadow(s: &Shadow) -> Fingerprint {
    // The shadow's own bounds already include the blur spread on the GPU
    // backends, but pad by the blur radius anyway — over-damage is cheap,
    // under-damage is a smear.
    let blur = s.blur_radius.0.ceil() as i32;
    Fingerprint::Shadow(ShadowFp {
        rect: paint_rect(&s.bounds, &s.content_mask, AA_PAD + blur.max(0)),
        bounds: s.bounds,
        mask: s.content_mask.bounds,
        color: HslaBits(s.color),
        blur: px_bits(s.blur_radius),
        corner_radii: corners_bits(&s.corner_radii),
    })
}

pub(crate) fn fingerprint_underline(u: &Underline) -> Fingerprint {
    Fingerprint::Underline(UnderlineFp {
        rect: paint_rect(&u.bounds, &u.content_mask, AA_PAD),
        bounds: u.bounds,
        mask: u.content_mask.bounds,
        color: HslaBits(u.color),
        thickness: px_bits(u.thickness),
        wavy: u.wavy,
    })
}

pub(crate) fn fingerprint_mono(s: &MonochromeSprite) -> Fingerprint {
    Fingerprint::Mono(MonoFp {
        rect: paint_rect(&s.bounds, &s.content_mask, AA_PAD),
        bounds: s.bounds,
        mask: s.content_mask.bounds,
        color: HslaBits(s.color),
        tile: s.tile,
        transformation: s.transformation,
    })
}

pub(crate) fn fingerprint_poly(s: &PolychromeSprite) -> Fingerprint {
    Fingerprint::Poly(PolyFp {
        rect: paint_rect(&s.bounds, &s.content_mask, AA_PAD),
        bounds: s.bounds,
        mask: s.content_mask.bounds,
        tile: s.tile,
        corner_radii: corners_bits(&s.corner_radii),
        opacity: s.opacity.to_bits(),
        grayscale: s.grayscale,
    })
}

pub(crate) fn fingerprint_path(p: &Path<ScaledPixels>) -> Fingerprint {
    let mut hash: u64 = 0xcbf29ce484222325;
    let mut digest = |v: f32| {
        for byte in v.to_bits().to_le_bytes() {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
    };
    for vertex in &p.vertices {
        digest(vertex.xy_position.x.0);
        digest(vertex.xy_position.y.0);
        digest(vertex.st_position.x);
        digest(vertex.st_position.y);
    }
    Fingerprint::Path(PathFp {
        rect: paint_rect(&p.bounds, &p.content_mask, AA_PAD),
        bounds: p.bounds,
        mask: p.content_mask.bounds,
        color: p.color,
        vertices: hash,
        vertex_count: p.vertices.len(),
    })
}

/// Capture the whole scene's fingerprints in paint (batch) order.
pub(crate) fn capture(scene: &Scene) -> Vec<Fingerprint> {
    use gpui::PrimitiveBatch;
    let mut out = Vec::new();
    for batch in scene.batches() {
        match batch {
            PrimitiveBatch::Shadows(range) => {
                out.extend(scene.shadows[range].iter().map(fingerprint_shadow))
            }
            PrimitiveBatch::Quads(range) => {
                out.extend(scene.quads[range].iter().map(fingerprint_quad))
            }
            PrimitiveBatch::Paths(range) => {
                out.extend(scene.paths[range].iter().map(fingerprint_path))
            }
            PrimitiveBatch::Underlines(range) => {
                out.extend(scene.underlines[range].iter().map(fingerprint_underline))
            }
            PrimitiveBatch::MonochromeSprites { range, .. } => out.extend(
                scene.monochrome_sprites[range]
                    .iter()
                    .map(fingerprint_mono),
            ),
            PrimitiveBatch::PolychromeSprites { range, .. } => out.extend(
                scene.polychrome_sprites[range]
                    .iter()
                    .map(fingerprint_poly),
            ),
            _ => {}
        }
    }
    out
}

/// Display-list diff: common prefix + common suffix; the damage is the
/// union of every remaining (changed/inserted/removed) primitive's rect
/// from both frames. `None` = frames identical, nothing to repaint.
pub(crate) fn diff(prev: &[Fingerprint], cur: &[Fingerprint]) -> Option<DamageRect> {
    let common = prev.len().min(cur.len());
    let mut prefix = 0;
    while prefix < common && prev[prefix] == cur[prefix] {
        prefix += 1;
    }
    if prefix == prev.len() && prefix == cur.len() {
        return None;
    }
    let mut suffix = 0;
    while suffix < common - prefix
        && prev[prev.len() - 1 - suffix] == cur[cur.len() - 1 - suffix]
    {
        suffix += 1;
    }

    let mut damage = DamageRect::EMPTY;
    for fp in &prev[prefix..prev.len() - suffix] {
        damage.union(fp.rect());
    }
    for fp in &cur[prefix..cur.len() - suffix] {
        damage.union(fp.rect());
    }
    (!damage.is_empty()).then_some(damage)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{point, px, size};

    fn quad(x: f32, y: f32, w: f32, h: f32, lightness: f32) -> Quad {
        let mut color = Hsla::default();
        color.l = lightness;
        Quad {
            bounds: Bounds {
                origin: point(ScaledPixels(x), ScaledPixels(y)),
                size: size(ScaledPixels(w), ScaledPixels(h)),
            },
            content_mask: ContentMask {
                bounds: Bounds {
                    origin: point(ScaledPixels(0.0), ScaledPixels(0.0)),
                    size: size(ScaledPixels(1000.0), ScaledPixels(1000.0)),
                },
            },
            background: Background::from(color),
            ..Default::default()
        }
    }

    #[test]
    fn identical_frames_produce_no_damage() {
        let a = vec![fingerprint_quad(&quad(10.0, 10.0, 50.0, 20.0, 0.5))];
        let b = vec![fingerprint_quad(&quad(10.0, 10.0, 50.0, 20.0, 0.5))];
        assert_eq!(diff(&a, &b), None);
        let _ = px(0.0); // keep the helper import exercised
    }

    #[test]
    fn moved_quad_damages_old_and_new_rects() {
        let a = vec![fingerprint_quad(&quad(10.0, 10.0, 50.0, 20.0, 0.5))];
        let b = vec![fingerprint_quad(&quad(200.0, 10.0, 50.0, 20.0, 0.5))];
        let d = diff(&a, &b).expect("damage");
        // Covers the vacated rect...
        assert!(d.x0 <= 10 - 2 && d.y0 <= 10 - 2);
        // ...and the new position.
        assert!(d.x1 >= 250 + 2 && d.y1 >= 30);
    }

    #[test]
    fn recolored_quad_in_a_long_list_damages_only_it() {
        let mut a = Vec::new();
        let mut b = Vec::new();
        for i in 0..100 {
            let y = i as f32 * 20.0;
            a.push(fingerprint_quad(&quad(0.0, y, 100.0, 18.0, 0.5)));
            let l = if i == 42 { 0.9 } else { 0.5 };
            b.push(fingerprint_quad(&quad(0.0, y, 100.0, 18.0, l)));
        }
        let d = diff(&a, &b).expect("damage");
        assert!(d.y0 >= 42 * 20 - 2, "damage starts at row 42: {d:?}");
        assert!(d.y1 <= 42 * 20 + 18 + 2, "damage ends after row 42: {d:?}");
    }

    #[test]
    fn inserted_primitive_damages_from_insertion_point() {
        let rows: Vec<Fingerprint> = (0..10)
            .map(|i| fingerprint_quad(&quad(0.0, i as f32 * 20.0, 100.0, 18.0, 0.5)))
            .collect();
        let mut with_insert = rows.clone();
        with_insert.insert(5, fingerprint_quad(&quad(0.0, 500.0, 40.0, 10.0, 0.1)));
        let d = diff(&rows, &with_insert).expect("damage");
        // Only the inserted rect differs (prefix 0..5, suffix 5..10 match).
        assert!(d.y0 >= 498 && d.y1 <= 512, "{d:?}");
    }
}
