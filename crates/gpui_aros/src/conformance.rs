//! Backend-porting conformance tests — host-runnable (`cargo test -p
//! gpui_aros` on any OS; no AROS, no window, no GPU).
//!
//! GPUI ships no test set for validating a *platform backend* (its
//! `TestPlatform` mocks rendering entirely; `VisualTestPlatform` is
//! macOS-only), so this suite is it: every renderer / input-translation
//! bug the AROS port hits in the field gets pinned here so no future port
//! change can silently regress it. See `PORTING.md` for the bug → test
//! ledger and the on-device checklist covering what host tests can't
//! (stack sizing, IntuiMessage quirks, LoadSeg).

use std::borrow::Cow;
use std::sync::Arc;

use gpui::{
    AtlasTile, Bounds, ContentMask, Corners, DevicePixels, Edges, Hsla, PlatformAtlas as _, Point,
    Quad, RenderSvgParams, ScaledPixels, Size, TransformationMatrix, linear_color_stop,
    linear_gradient, rgb, solid_background,
};

use crate::atlas::CpuAtlas;
use crate::renderer::CpuRenderer;

// ---- helpers ---------------------------------------------------------------

fn sp(v: f32) -> ScaledPixels {
    ScaledPixels(v)
}

fn bounds(x: f32, y: f32, w: f32, h: f32) -> Bounds<ScaledPixels> {
    Bounds {
        origin: Point { x: sp(x), y: sp(y) },
        size: Size {
            width: sp(w),
            height: sp(h),
        },
    }
}

/// A content mask covering the whole (small) test framebuffer.
fn full_mask() -> ContentMask<ScaledPixels> {
    ContentMask {
        bounds: bounds(0., 0., 4096., 4096.),
    }
}

fn renderer(w: i32, h: i32) -> CpuRenderer {
    CpuRenderer::new(Size {
        width: DevicePixels(w),
        height: DevicePixels(h),
    })
}

/// Premultiplied RGBA of the framebuffer pixel at (x, y).
fn px(r: &CpuRenderer, x: u32, y: u32) -> [u8; 4] {
    let fb = r.framebuffer();
    let o = ((y * r.width() + x) * 4) as usize;
    [fb[o], fb[o + 1], fb[o + 2], fb[o + 3]]
}

/// Insert a monochrome (1 byte/px alpha) tile of constant `coverage` into
/// the atlas — the same route `Window::paint_svg` uses, keyed as an SVG
/// raster.
fn insert_mono_tile(atlas: &Arc<CpuAtlas>, name: &str, w: i32, h: i32, coverage: u8) -> AtlasTile {
    let params = RenderSvgParams {
        path: name.to_string().into(),
        size: Size {
            width: DevicePixels(w),
            height: DevicePixels(h),
        },
    };
    atlas
        .get_or_insert_with(&params.into(), &mut || {
            Ok(Some((
                Size {
                    width: DevicePixels(w),
                    height: DevicePixels(h),
                },
                Cow::Owned(vec![coverage; (w * h) as usize]),
            )))
        })
        .unwrap()
        .unwrap()
}

fn mono_sprite(tile: AtlasTile, target: Bounds<ScaledPixels>, color: Hsla) -> gpui::MonochromeSprite {
    gpui::MonochromeSprite {
        order: 0,
        pad: 0,
        bounds: target,
        content_mask: full_mask(),
        color,
        tile,
        transformation: TransformationMatrix::unit(),
    }
}

fn plain_quad(b: Bounds<ScaledPixels>, background: gpui::Background) -> Quad {
    Quad {
        order: 0,
        border_style: Default::default(),
        bounds: b,
        content_mask: full_mask(),
        background,
        border_color: Hsla::transparent_black(),
        corner_radii: Corners::default(),
        border_widths: Edges::default(),
    }
}

// ---- monochrome sprites (glyphs + SVG icons) --------------------------------
//
// Field bug (AROS, 2026-07-04): gpui rasterizes SVGs at 2× the layout size
// (SMOOTH_SVG_SCALE_FACTOR) and GPU backends *sample* the tile into the
// paint bounds; the CPU renderer blitted tile pixels 1:1, so every icon
// drew at double size and got clipped ("too big and half cut").

#[test]
fn svg_sprite_downsamples_into_bounds() {
    let mut r = renderer(16, 16);
    let atlas = r.sprite_atlas_concrete();
    // 8×8 tile (the "2× raster"), painted into a 4×4 target — like every
    // SVG icon.
    let tile = insert_mono_tile(&atlas, "icons/test-downsample.svg", 8, 8, 255);
    let white = Hsla::from(rgb(0xffffff));
    r.draw_monochrome_sprite_for_test(&mono_sprite(tile, bounds(2., 2., 4., 4.), white));

    // Inside the target bounds: covered.
    assert_eq!(px(&r, 3, 3)[3], 255, "inside the bounds must be painted");
    assert_eq!(px(&r, 5, 5)[3], 255, "inside the bounds must be painted");
    // Where the OLD 1:1 blit would have spilled (tile is 8 wide from x=2):
    // x in 6..10 must stay untouched.
    assert_eq!(px(&r, 7, 3)[3], 0, "must not spill past bounds (old 2× bug)");
    assert_eq!(px(&r, 9, 9)[3], 0, "must not spill past bounds (old 2× bug)");
}

#[test]
fn glyph_sprite_blits_one_to_one() {
    let mut r = renderer(16, 16);
    let atlas = r.sprite_atlas_concrete();
    let tile = insert_mono_tile(&atlas, "icons/test-glyph.svg", 4, 4, 255);
    let white = Hsla::from(rgb(0xffffff));
    r.draw_monochrome_sprite_for_test(&mono_sprite(tile, bounds(6., 6., 4., 4.), white));

    assert_eq!(px(&r, 6, 6)[3], 255);
    assert_eq!(px(&r, 9, 9)[3], 255);
    assert_eq!(px(&r, 5, 6)[3], 0);
    assert_eq!(px(&r, 10, 9)[3], 0);
}

#[test]
fn monochrome_tint_is_premultiplied() {
    let mut r = renderer(8, 8);
    let atlas = r.sprite_atlas_concrete();
    // Half coverage, pure red tint → premultiplied red ≈ alpha ≈ 128.
    let tile = insert_mono_tile(&atlas, "icons/test-tint.svg", 4, 4, 128);
    let red = Hsla::from(rgb(0xff0000));
    r.draw_monochrome_sprite_for_test(&mono_sprite(tile, bounds(0., 0., 4., 4.), red));

    let p = px(&r, 1, 1);
    assert!(
        (p[3] as i32 - 128).abs() <= 2,
        "alpha should be ~coverage, got {}",
        p[3]
    );
    assert!(
        (p[0] as i32 - p[3] as i32).abs() <= 2,
        "premultiplied red should equal alpha, got r={} a={}",
        p[0],
        p[3]
    );
    assert_eq!(p[1], 0, "green must stay 0 under a red tint");
}

// ---- quads: solid, gradient, content mask -----------------------------------

#[test]
fn solid_quad_fills_and_respects_content_mask() {
    let mut r = renderer(16, 16);
    let mut quad = plain_quad(bounds(0., 0., 16., 16.), solid_background(rgb(0x00ff00)));
    // Mask off everything right of x=8.
    quad.content_mask = ContentMask {
        bounds: bounds(0., 0., 8., 16.),
    };
    r.draw_quad_for_test(&quad);

    assert_eq!(px(&r, 4, 8)[1], 255, "inside the mask: filled green");
    assert_eq!(px(&r, 12, 8)[1], 0, "outside the mask: untouched");
}

#[test]
fn linear_gradient_endpoint_colors() {
    let mut r = renderer(8, 32);
    // 180° = top-to-bottom (CSS convention): red at the start, blue at the end.
    let bg = linear_gradient(
        180.,
        linear_color_stop(rgb(0xff0000), 0.),
        linear_color_stop(rgb(0x0000ff), 1.),
    );
    r.draw_quad_for_test(&plain_quad(bounds(0., 0., 8., 32.), bg));

    let top = px(&r, 4, 1);
    let bottom = px(&r, 4, 30);
    assert!(top[0] > 200 && top[2] < 60, "top should be red, got {top:?}");
    assert!(
        bottom[2] > 200 && bottom[0] < 60,
        "bottom should be blue, got {bottom:?}"
    );
}

#[test]
fn rounded_quad_leaves_corners_unpainted() {
    let mut r = renderer(16, 16);
    let mut quad = plain_quad(bounds(0., 0., 16., 16.), solid_background(rgb(0xffffff)));
    quad.corner_radii = Corners {
        top_left: sp(8.),
        top_right: sp(8.),
        bottom_right: sp(8.),
        bottom_left: sp(8.),
    };
    r.draw_quad_for_test(&quad);

    assert_eq!(px(&r, 0, 0)[3], 0, "fully-rounded corner pixel stays empty");
    assert!(px(&r, 8, 8)[3] > 200, "center is filled");
}

// ---- atlas -------------------------------------------------------------------

#[test]
fn atlas_roundtrips_bytes_and_reuses_keys() {
    let atlas = Arc::new(CpuAtlas::new());
    let t1 = insert_mono_tile(&atlas, "icons/test-roundtrip.svg", 3, 2, 7);
    let read = atlas.read_tile(&t1).expect("tile readable");
    assert_eq!(read.width, 3);
    assert_eq!(read.height, 2);
    assert_eq!(read.bytes_per_pixel, 1);
    assert!(read.data.iter().all(|&b| b == 7));

    // Same key → same tile, no new allocation.
    let t2 = insert_mono_tile(&atlas, "icons/test-roundtrip.svg", 3, 2, 7);
    assert_eq!(t1.tile_id, t2.tile_id);
    assert_eq!(t1.texture_id, t2.texture_id);
}

// ---- input translation (rawkey → GPUI) ---------------------------------------
//
// Field bugs (AROS, 2026-07-04): dead-key IAddress deref crashed on
// injected input (fixed in the C glue — not host-testable); these pin the
// pure mapping layer that surrounds it.

#[test]
fn rawkey_named_keys_match_backend_vocabulary() {
    use crate::input::key_name;
    for (code, name) in [
        (0x40, "space"),
        (0x41, "backspace"),
        (0x42, "tab"),
        (0x44, "enter"),
        (0x43, "enter"), // keypad enter folds into enter
        (0x45, "escape"),
        (0x46, "delete"),
        (0x48, "pageup"),
        (0x49, "pagedown"),
        (0x4C, "up"),
        (0x4D, "down"),
        (0x4E, "right"),
        (0x4F, "left"),
        (0x50, "f1"),
        (0x59, "f10"),
        (0x4B, "f11"),
        (0x6F, "f12"),
    ] {
        assert_eq!(
            key_name(code, b"\0").as_deref(),
            Some(name),
            "rawkey {code:#x}"
        );
    }
}

#[test]
fn printable_keys_come_from_the_keymap_lowercased() {
    use crate::input::key_name;
    // A French layout's physical Q key maps to 'a' — the binding name must
    // follow the keymap, not the raw code.
    assert_eq!(key_name(0x10, b"a\0").as_deref(), Some("a"));
    assert_eq!(key_name(0x10, b"A\0").as_deref(), Some("a"), "capslock-proof");
    // Unmapped code with no chars: nothing sensible to report.
    assert_eq!(key_name(0x7F, b"\0"), None);
}

#[test]
fn qualifier_bits_map_to_gpui_modifiers() {
    use crate::input::*;
    let m = modifiers_from_qualifier(IEQUALIFIER_LSHIFT | IEQUALIFIER_CONTROL);
    assert!(m.shift && m.control && !m.alt && !m.platform);

    // Amiga/Command keys = the platform modifier (Cmd/Win equivalent).
    let m = modifiers_from_qualifier(IEQUALIFIER_RCOMMAND);
    assert!(m.platform && !m.control);

    let m = modifiers_from_qualifier(IEQUALIFIER_LALT | IEQUALIFIER_RALT);
    assert!(m.alt);
}

#[test]
fn latin1_decodes_and_stops_at_nul() {
    use crate::input::latin1_to_string;
    assert_eq!(latin1_to_string(b"ab\0zz").as_deref(), Some("ab"));
    assert_eq!(latin1_to_string(b"\0"), None);
    // 0xE9 = é in ISO-8859-1: maps 1:1 onto the Unicode scalar.
    assert_eq!(latin1_to_string(&[0xE9, 0]).as_deref(), Some("é"));
}
