# GPUI backend-porting conformance suite

Upstream GPUI has **no test set for validating a platform backend**: its
`TestPlatform` mocks rendering entirely, `VisualTestPlatform` is
"macOS-only for now", and the linux/windows backends ship zero tests. When
the AROS port hit its first field bugs, nothing existed to pin them — so
this suite is that test set, started 2026-07-04.

## How it works

- **Host-runnable**: `cargo test -p gpui_aros` on any OS — no AROS, no
  window, no GPU. The crate is split so the portable core (`atlas`,
  `renderer`, `input`) compiles everywhere; only the AROS shell (`glue`,
  `window`, `platform`, `dispatcher`, `display`, `text`) is cfg-gated
  behind `target_os = "aros"` (its `gpa_*` externs resolve only at the
  collect-aros link).
- **Grows with the port**: every renderer / input-translation bug found in
  the field gets a pinning test here before (or with) its fix. If it can't
  be host-tested (OS-side behavior), it goes on the on-device checklist
  below instead.
- Tests live in `src/conformance.rs` and drive the CPU renderer / atlas /
  input mapping directly — primitives in, pixels (or key names) out.

## Bug → test ledger

| Field bug (all found on booted AROS) | Pinned by |
| --- | --- |
| SVG icons drew at 2× and got clipped — gpui rasterizes SVGs at `SMOOTH_SVG_SCALE_FACTOR` (2×) and GPU backends *sample* the tile into the paint bounds; the CPU renderer blitted 1:1 | `svg_sprite_downsamples_into_bounds`, `glyph_sprite_blits_one_to_one` |
| Glyph tint must premultiply (renderer feeds an opaque RECTFMT_RGBA blit) | `monochrome_tint_is_premultiplied` |
| Gradient quads invisible until `Background::as_linear_gradient()` existed; CSS angle convention easy to flip | `linear_gradient_endpoint_colors` |
| Content-mask clipping is the only thing between a sprite and neighboring UI | `solid_quad_fills_and_respects_content_mask`, `rounded_quad_leaves_corners_unpainted` |
| Atlas keyed by `AtlasKey` — re-inserting a key must reuse the tile, bytes must round-trip | `atlas_roundtrips_bytes_and_reuses_keys` |
| Keyboard: key names must match the shared backend vocabulary or every keymap silently misses | `rawkey_named_keys_match_backend_vocabulary` |
| Layout-aware printables (French `a` on physical Q) + capslock-proof binding identity | `printable_keys_come_from_the_keymap_lowercased` |
| Amiga/Command → `platform`, `secondary-` stays Ctrl off-macOS | `qualifier_bits_map_to_gpui_modifiers` |
| FTXT / keymap bytes are ISO-8859-1, NUL-terminated | `latin1_decodes_and_stops_at_nul` |

## On-device checklist (not host-testable)

Run on booted AROS via `graft/aros-ctl` (see `gpui_aros_smoke`):

- [ ] **Stack**: launch with `Stack 16000000`. Shell default (~tens of KB)
      overflows under gpui's dispatch recursion and — single address
      space — corrupts *other* tasks (emul-handler, graphics.library
      crash with wild NULL-offset faults pointing nowhere near the app).
- [ ] **IntuiMessage quirks**: never dereference `IAddress` on RAWKEY —
      injected input (cocoametal FIFO → keyboard HIDD) delivers
      non-pointer values there (crashed at NULL+0x28 in the field).
      Dead-key composition is disabled rather than crashy.
- [ ] **Window + input smoke**: `C:GpuiSmoke` renders the primitive
      gallery and echoes keystrokes (`last key: …`); survives mouse
      clicks and held keys.
- [ ] **LoadSeg scale**: strip the binary (`llvm-strip --strip-debug`) —
      a debug-info ET_REL takes minutes to relocate; and never boot with
      a `-DDEBUG` dos.library for real runs (it logs every packet).
- [ ] Clipboard round-trip (`aros-ctl cmdc/cmdv` once wired), window
      resize via the size gadget, scroll wheel (NewMouse rawkeys).

## Running

```sh
cargo test -p gpui_aros                    # host: the conformance suite
scripts/check-aros.sh                      # (Feraille repo) full-app AROS check
crates/gpui_aros_smoke/link-aros.sh        # on-device smoke build
```
