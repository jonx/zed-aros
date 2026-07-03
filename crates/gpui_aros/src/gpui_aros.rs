//! GPUI platform backend for AROS (hosted on darwin-aarch64).
//!
//! A CPU-rendered backend: Intuition windows via the C glue in
//! `c/gpui_aros_glue.c`, a tiny-skia software rasterizer for the GPUI scene,
//! and `gpui_wgpu::CosmicTextSystem` for text (OS-independent).
