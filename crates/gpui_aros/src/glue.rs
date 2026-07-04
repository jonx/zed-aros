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

/// Mouse-button codes carried in `GpaEvent::code` for mousedown/up.
pub(crate) const GPA_BUTTON_LEFT: c_int = 0;
pub(crate) const GPA_BUTTON_RIGHT: c_int = 1;
pub(crate) const GPA_BUTTON_MIDDLE: c_int = 2;

// -- Rawkey constants (devices/inputevent.h + devices/rawkeycodes.h) --------
// Mirrored here rather than bound: they are stable Amiga ABI, fixed since
// the 1.x days, and AROS's virtual rawkey set is MorphOS-compatible.

/// `IECODE_UP_PREFIX`: set in `GpaEvent::code` for key releases.
pub(crate) const IECODE_UP_PREFIX: c_int = 0x80;

/// `IEQUALIFIER_*` bits carried in `GpaEvent::qualifier`.
pub(crate) const IEQUALIFIER_LSHIFT: c_int = 1 << 0;
pub(crate) const IEQUALIFIER_RSHIFT: c_int = 1 << 1;
pub(crate) const IEQUALIFIER_CAPSLOCK: c_int = 1 << 2;
pub(crate) const IEQUALIFIER_CONTROL: c_int = 1 << 3;
pub(crate) const IEQUALIFIER_LALT: c_int = 1 << 4;
pub(crate) const IEQUALIFIER_RALT: c_int = 1 << 5;
pub(crate) const IEQUALIFIER_LCOMMAND: c_int = 1 << 6;
pub(crate) const IEQUALIFIER_RCOMMAND: c_int = 1 << 7;
pub(crate) const IEQUALIFIER_REPEAT: c_int = 1 << 9;

/// The modifier keys' own rawkey codes (LSHIFT..RAMIGA) — state-only, they
/// update `Modifiers` but never emit a Key event.
pub(crate) const RAWKEY_MODIFIER_FIRST: c_int = 0x60;
pub(crate) const RAWKEY_MODIFIER_LAST: c_int = 0x67;

/// NewMouse standard: the scroll wheel arrives as rawkey codes.
pub(crate) const RAWKEY_NM_WHEEL_UP: c_int = 0x7A;
pub(crate) const RAWKEY_NM_WHEEL_DOWN: c_int = 0x7B;
pub(crate) const RAWKEY_NM_WHEEL_LEFT: c_int = 0x7C;
pub(crate) const RAWKEY_NM_WHEEL_RIGHT: c_int = 0x7D;
pub(crate) const RAWKEY_NM_BUTTON_FOURTH: c_int = 0x7E;

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
}
