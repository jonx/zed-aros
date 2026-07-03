//! Rust declarations for the flat C glue in `c/gpui_aros_glue.c`.
//!
//! The glue object is compiled by `build.rs` when targeting AROS with the SDK
//! headers present, otherwise linked in at final-link time by collect-aros.
//! `cargo check` never links, so these unresolved externs are fine for the
//! check milestone.

use std::os::raw::{c_char, c_int, c_uint, c_void};

/// Mirrors `struct GpaEvent` in the C glue. Event kinds below.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub(crate) struct GpaEvent {
    pub kind: c_int,
    pub code: c_int,
    pub qualifier: c_int,
    pub x: c_int,
    pub y: c_int,
}

pub(crate) const GPA_EVENT_CLOSE: c_int = 1;
pub(crate) const GPA_EVENT_NEWSIZE: c_int = 2;
pub(crate) const GPA_EVENT_REFRESH: c_int = 3;
pub(crate) const GPA_EVENT_MOUSEMOVE: c_int = 4;
pub(crate) const GPA_EVENT_MOUSEDOWN: c_int = 5;
pub(crate) const GPA_EVENT_MOUSEUP: c_int = 6;
pub(crate) const GPA_EVENT_RAWKEY: c_int = 7;

/// Mouse-button codes carried in `GpaEvent::code` for mousedown/up.
pub(crate) const GPA_BUTTON_LEFT: c_int = 0;
pub(crate) const GPA_BUTTON_RIGHT: c_int = 1;
pub(crate) const GPA_BUTTON_MIDDLE: c_int = 2;

unsafe extern "C" {
    pub(crate) fn gpa_open_window(
        x: c_int,
        y: c_int,
        inner_w: c_int,
        inner_h: c_int,
        title: *const c_char,
    ) -> *mut c_void;

    pub(crate) fn gpa_close_window(handle: *mut c_void);

    pub(crate) fn gpa_blit(
        handle: *mut c_void,
        rgba: *const c_void,
        src_stride_bytes: c_int,
        x: c_int,
        y: c_int,
        width: c_int,
        height: c_int,
    );

    pub(crate) fn gpa_poll_event(handle: *mut c_void, out: *mut GpaEvent) -> c_int;

    pub(crate) fn gpa_window_sigmask(handle: *mut c_void) -> c_uint;

    pub(crate) fn gpa_wait_timeout_ms(mask: c_uint, ms: c_int);

    pub(crate) fn gpa_inner_size(handle: *mut c_void, out_w: *mut c_int, out_h: *mut c_int);

    pub(crate) fn gpa_set_title(handle: *mut c_void, title: *const c_char);

    pub(crate) fn gpa_screen_size(out_w: *mut c_int, out_h: *mut c_int) -> c_int;
}
