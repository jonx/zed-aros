# gpui_aros — takeover guide

A GPUI platform backend for **AROS** (the open-source AmigaOS), hosted on
darwin-aarch64. It lets any GPUI app (Zed's UI framework) run on AROS with a
software CPU renderer. Built for the Feraille file-manager port, but nothing
here is Feraille-specific.

**Status: working.** A real GPUI app opens an Intuition window, renders the full
UI (quads, shadows, paths, glyphs, images) through a tiny-skia CPU rasterizer,
takes keyboard/mouse/wheel input, and reads/writes `clipboard.device`. It is
**not feature-complete** — see [What's missing](#whats-missing).

## Where it lives

- This crate: `~/Source/zed-aros/crates/gpui_aros`, branch `aros-platform`, a
  fork of Zed pinned at rev `1d217ee` (Feraille's gpui rev).
- One `cfg(target_os = "aros")` arm in `gpui_platform::current_platform()`
  selects `ArosPlatform`. gpui core is **unmodified**.
- Consumers point their gpui git dep at this fork via Cargo `[patch]` (Feraille
  does this in its root `Cargo.toml`).
- **Nothing is pushed to any upstream** — all local until the owner decides.

## The one big idea

gpui core needs only two things from a platform: a `PlatformDispatcher` and a
renderer that consumes a `Scene`. gpui core's *only* async dep is `async-task`
(pure Rust, no reactor). Everything else that looked hard — the POSIX
event-loop/syscall stack (`polling`/`rustix`/`errno`), `libc` — is pulled by
**sibling crates** (`util` via `smol`, `http_client` via `async-tar`), not gpui
itself. So the port is "trim two crates + write a CPU backend," not "port the
Rust ecosystem to AROS." Keep that framing; it's why this was feasible.

## File map

| File | Lines | What |
|---|---|---|
| `src/gpui_aros.rs` | ~40 | module root, `pub use platform::ArosPlatform` |
| `src/platform.rs` | ~370 | `ArosPlatform` — run loop, ~45 `Platform` methods |
| `src/window.rs` | ~580 | `ArosWindow` — Intuition window, callbacks, `draw`, event pump |
| `src/renderer.rs` | ~720 | `CpuRenderer` — tiny-skia rasterizer, one arm per `PrimitiveBatch` |
| `src/atlas.rs` | ~260 | `CpuAtlas` — etagere shelf-packing over `Vec<u8>` textures |
| `src/dispatcher.rs` | ~100 | std-thread background pool + main-thread queue |
| `src/display.rs` | ~55 | one synthetic display from `gpa_screen_size` |
| `src/input.rs` | ~110 | RAWKEY → `Keystroke`, qualifier → `Modifiers`, mouse |
| `src/text.rs` | ~30 | re-exports `gpui_wgpu::CosmicTextSystem` (reused as-is) |
| `src/glue.rs` | ~95 | `extern "C"` decls for the C glue |
| `src/conformance.rs` | ~310 | renderer conformance checks |
| `c/gpui_aros_glue.c` | ~300 | flat C over Intuition/CyberGraphics/clipboard.device |
| `build.rs` | — | compiles the glue **iff** `target_os=aros` and SDK headers exist |

### The C glue surface (`gpa_*`)

The Rust↔AROS boundary is one flat C file. Current entry points:

`gpa_init` / `gpa_init_main` · `gpa_open_window` / `gpa_close_window` ·
`gpa_blit` (RGBA→RastPort via cybergraphics `WritePixelArray`, `RECTFMT_RGBA`
matches tiny-skia) · `gpa_poll_event` (IDCMP → repr(C) `GpaEvent`) ·
`gpa_map_rawkey` (keymap.library) · `gpa_inner_size` · `gpa_set_title` ·
`gpa_screen_size` · `gpa_window_sigmask` / `gpa_wait_timeout_ms` /
`gpa_wake_main` / `gpa_wake_sigmask` (event-loop wakeup) ·
`gpa_clipboard_read_text` / `gpa_clipboard_write_text` (clipboard.device).

`build.rs` compiles the glue only when the AROS SDK headers are present at
`$AROS_BUILD/bin/darwin-aarch64/gen/include` (default `~/aros-build`); otherwise
it defers to link time so plain `cargo check` still passes. For a runnable
binary the glue is archived into the staticlib and picked up by `collect-aros`
(see Feraille's `link-aros.sh`).

## What's implemented (solid)

- **Renderer** — `PrimitiveBatch` arms for Quads (rounded corners + borders),
  Shadows, Paths, Underlines, MonochromeSprites (glyphs), PolychromeSprites
  (images). Content-mask clipping honored. `conformance.rs` guards it.
- **Text** — `gpui_wgpu::CosmicTextSystem` (cosmic-text/swash), full shaping.
  OS-independent; do not fork it.
- **Input** — keyboard (RAWKEY→keymap→`Keystroke`), modifiers from qualifier,
  mouse buttons, scroll wheel.
- **Clipboard** — real `clipboard.device` read + write.
- **Dispatcher** — std-thread background pool; main-thread queue drained by the
  run loop; `gpa_wake_main` signals it.

## What's missing

Prioritised for a new owner:

1. **Native menus** — `Platform::set_menus` is a no-op. Wire `Menu`/`MenuItem`
   → Intuition `SetMenuStrip`; map `IDCMP_MENUPICK` back to actions.
2. **File requesters** — `prompt_for_paths` / `prompt_for_new_path` return
   `Ok(None)`. Wire to `asl.library` (`AslRequest`, file + dir modes).
3. **Cursor** — `set_cursor_style` no-op. Map `CursorStyle` → Intuition pointer
   (`SetWindowPointer` / a sprite set).
4. **`open_url`** no-op; window **resize** is best-effort no-op.
5. **Renderer tail** — pattern fills (slash/checkerboard), per-edge border
   widths, sprite rotation (`transformation`), `Surfaces` (video). All rare in
   chrome; grep `TODO` in `renderer.rs`.
6. **Subpixel AA** — deliberately off (`is_subpixel_rendering_supported=false`).
7. **GPU acceleration** — none; pure software. See the
   [gpufx.library plan](../../../aros-aarch64/docs/features/gpufx/README.md).

## Build / check / run

```sh
# type-check for AROS (no link):
cargo +nightly-2026-06-27 check -p gpui_aros \
  --target ~/Source/aros-aarch64/hosted/rust/aarch64-unknown-aros.json \
  -Zjson-target-spec -Zbuild-std=std,panic_abort

# the AROS SDK + toolchain (stable, out of /tmp):
#   ~/aros-build  ~/aros-crosstools   (rebuild: aros-aarch64/graft/rebuild-aros.sh)
# run a real app on booted AROS: see Feraille's link-aros.sh + graft/aros-ctl.
```

The pinned nightly matters — the custom Rust std for AROS rides the toolchain's
`rust-src` symlink to `~/Source/rust-aros`.

## The dependency-trim story (don't relearn this the hard way)

To make gpui + its workspace compile for `target_os=aros`, these are gated /
patched (all inert off-AROS):
- `util`: `smol`, `dirs`, `which` moved off the AROS graph (they pull the
  reactor / assume unix-PATH). `paths.rs` gets a `$HOME` fallback.
- `http_client`: `async-tar` + the `github_download` module gated off.
- `scheduler`: `flume` without `eventual-fairness` (that pulled getrandom 0.2).
- `[patch.crates-io]`: vendored `stacker` (no-op AROS stack growth) and
  `filetime` (AROS arm). See `vendor-aros/`.
- gpui-component: a worktree swaps `smol::channel`→`async-channel`
  (`~/Source/gpui-component-aros`).

## Proposed next architecture: an `aros` platform-crate family

Today the AROS API surface is ad-hoc C glue duplicated per consumer (this
crate's `gpui_aros_glue.c`, the rust-aros std `aros_*_glue.c`, and — when it
grows — `feraille-shell-aros`). **Consolidate into reusable crates** so the
backend and the app share one binding layer:

- **`aros-sys`** — raw `extern "C"` bindings + the C shim for AROS libraries:
  exec, dos, intuition, graphics, layers, cybergraphics, keymap, asl,
  workbench, icon, datatypes, and `clipboard.device`. Unsafe, thin.
- **`aros`** — safe Rust wrappers: `Screen`, `Window`, `RastPort`/blit,
  `MenuStrip`, `FileRequester`, `Clipboard`, `Icon`, `DataType`. RAII handles,
  `Result` errors, `-ffixed-x18`-safe call convention.

Then `gpui_aros` builds its window/menu/dialog/cursor/clipboard on `aros`
instead of private glue, and `feraille-shell-aros` uses the same crate for
reveal/icons/thumbnails. This is the natural home for items 1–4 above. Keep the
crates in this fork (or a small standalone repo) until the port stabilises.
