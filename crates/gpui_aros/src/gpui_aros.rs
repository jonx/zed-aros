//! GPUI platform backend for AROS (hosted on darwin-aarch64).
//!
//! A CPU-rendered backend: Intuition windows via the C glue in
//! `c/gpui_aros_glue.c`, a tiny-skia software rasterizer for the GPUI scene,
//! and `gpui_wgpu::CosmicTextSystem` for text (OS-independent).
//!
//! ## Module split: portable core vs AROS shell
//!
//! `atlas`, `renderer`, and `input` are pure Rust with no OS dependency —
//! they compile (and their tests run) on **any host**, forming the
//! backend-porting conformance suite (see `PORTING.md`): every renderer /
//! input-translation bug found in the field on AROS gets pinned by a
//! host-runnable test here. The AROS-only shell (`glue`, `window`,
//! `platform`, `dispatcher`, `display`, `text`) is cfg-gated: it references
//! `gpa_*` externs that only resolve when collect-aros links the final
//! binary, which would break `cargo test` on the host.

mod atlas;
mod damage;
pub mod input;
mod renderer;

#[cfg(test)]
mod conformance;

#[cfg(target_os = "aros")]
mod dispatcher;
#[cfg(target_os = "aros")]
mod display;
#[cfg(target_os = "aros")]
mod glue;
#[cfg(target_os = "aros")]
mod menus;
#[cfg(target_os = "aros")]
mod platform;
#[cfg(target_os = "aros")]
mod text;
#[cfg(target_os = "aros")]
mod window;

#[cfg(target_os = "aros")]
pub use platform::ArosPlatform;
