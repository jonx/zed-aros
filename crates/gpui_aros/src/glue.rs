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
    /// RAWKEY only: keymap.library translation of the key, NUL-terminated,
    /// system charset (ISO-8859-1 on stock AROS). `chars` = with the real
    /// qualifiers (what was typed); `base_chars` = with qualifier 0 (the
    /// unmodified key, for the keybinding name).
    pub chars: [u8; 8],
    pub base_chars: [u8; 8],
}

pub(crate) const GPA_EVENT_CLOSE: c_int = 1;
pub(crate) const GPA_EVENT_NEWSIZE: c_int = 2;
pub(crate) const GPA_EVENT_REFRESH: c_int = 3;
pub(crate) const GPA_EVENT_MOUSEMOVE: c_int = 4;
pub(crate) const GPA_EVENT_MOUSEDOWN: c_int = 5;
pub(crate) const GPA_EVENT_MOUSEUP: c_int = 6;
pub(crate) const GPA_EVENT_RAWKEY: c_int = 7;
/// `code` = the item id passed to `gpa_set_menus` (an index into the
/// platform's action registry).
pub(crate) const GPA_EVENT_MENUPICK: c_int = 8;

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

    /// Record the main task + allocate the wake signal. Main thread, once,
    /// before the run loop.
    pub(crate) fn gpa_init_main();

    /// The wake signal's mask (0 before `gpa_init_main`); OR it into the
    /// run loop's park mask.
    pub(crate) fn gpa_wake_sigmask() -> c_uint;

    /// Nudge the run loop out of its park. Any thread; no-op before init.
    pub(crate) fn gpa_wake_main();

    /// Write `len` bytes to clipboard.device unit 0 as FORM FTXT / CHRS
    /// (system charset). Main thread. Returns 0 on success.
    pub(crate) fn gpa_clipboard_write_text(bytes: *const c_void, len: c_int) -> c_int;

    /// Read the first CHRS chunk of the current FTXT clip into an
    /// AllocVec'd NUL-terminated buffer (`gpa_free` it). Main thread.
    /// Returns 0 on success.
    pub(crate) fn gpa_clipboard_read_text(out: *mut *mut c_void, out_len: *mut c_int) -> c_int;

    /// Free a buffer handed out by the glue (AllocVec-backed).
    pub(crate) fn gpa_free(p: *mut c_void);

    /// Programmatic inner-size change (ChangeWindowBox); Intuition answers
    /// asynchronously with a NEWSIZE event.
    pub(crate) fn gpa_set_size(handle: *mut c_void, inner_w: c_int, inner_h: c_int);

    /// Replace the window's menu strip with the flattened spec (gadtools
    /// NewMenu built + laid out natively). Empty count clears the strip.
    pub(crate) fn gpa_set_menus(
        handle: *mut c_void,
        specs: *const GpaMenuSpec,
        count: c_int,
    ) -> c_int;

    /// Set one of the shared pointerclass pointers (GPA_PTR_* in the glue);
    /// 0 restores the Intuition default arrow.
    pub(crate) fn gpa_set_pointer(handle: *mut c_void, style: c_int);

    /// Blocking asl.library file requester. On success (0) hands out an
    /// AllocVec'd NUL-separated, double-NUL-terminated path list
    /// (`gpa_free` it). Cancel/failure returns -1.
    pub(crate) fn gpa_asl_request_paths(
        save_mode: c_int,
        drawers_only: c_int,
        multiselect: c_int,
        initial_drawer: *const c_char,
        initial_file: *const c_char,
        title: *const c_char,
        out: *mut *mut c_void,
        out_len: *mut c_int,
    ) -> c_int;
}

/// Mirrors `struct GpaMenuSpec` in the C glue: one flattened menu entry.
#[repr(C)]
pub(crate) struct GpaMenuSpec {
    /// 0 = menu title, 1 = item, 2 = sub-item.
    pub level: c_int,
    /// Action-registry index; -1 = separator / not pickable.
    pub id: c_int,
    /// System charset (ISO-8859-1), NUL-terminated.
    pub label: *const c_char,
    /// Single-char Amiga command key, or null.
    pub commkey: *const c_char,
    pub disabled: c_int,
    pub checked: c_int,
}
