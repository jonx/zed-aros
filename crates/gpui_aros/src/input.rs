//! Rawkey → GPUI input translation: the pure, host-testable half of the
//! AROS keyboard/wheel pipeline (the AROS-only half — IntuiMessage
//! draining and keymap.library character translation — lives in `glue.rs`
//! / `window.rs`).
//!
//! The constants mirror `devices/inputevent.h` + `devices/rawkeycodes.h`:
//! stable Amiga ABI, fixed since the 1.x days; AROS's virtual rawkey set is
//! MorphOS-compatible. Verified against the AROS source
//! (`compiler/include/devices/{inputevent,rawkeycodes}.h`).

use std::os::raw::c_int;

use gpui::Modifiers;

/// `IECODE_UP_PREFIX`: set in the rawkey code for key releases.
pub const IECODE_UP_PREFIX: c_int = 0x80;

/// `IEQUALIFIER_*` bits carried in the IntuiMessage qualifier.
pub const IEQUALIFIER_LSHIFT: c_int = 1 << 0;
pub const IEQUALIFIER_RSHIFT: c_int = 1 << 1;
pub const IEQUALIFIER_CAPSLOCK: c_int = 1 << 2;
pub const IEQUALIFIER_CONTROL: c_int = 1 << 3;
pub const IEQUALIFIER_LALT: c_int = 1 << 4;
pub const IEQUALIFIER_RALT: c_int = 1 << 5;
pub const IEQUALIFIER_LCOMMAND: c_int = 1 << 6;
pub const IEQUALIFIER_RCOMMAND: c_int = 1 << 7;
pub const IEQUALIFIER_REPEAT: c_int = 1 << 9;

/// The modifier keys' own rawkey codes (LSHIFT..RAMIGA) — state-only, they
/// update `Modifiers` but never emit a Key event.
pub const RAWKEY_MODIFIER_FIRST: c_int = 0x60;
pub const RAWKEY_MODIFIER_LAST: c_int = 0x67;

/// NewMouse standard: the scroll wheel arrives as rawkey codes.
pub const RAWKEY_NM_WHEEL_UP: c_int = 0x7A;
pub const RAWKEY_NM_WHEEL_DOWN: c_int = 0x7B;
pub const RAWKEY_NM_WHEEL_LEFT: c_int = 0x7C;
pub const RAWKEY_NM_WHEEL_RIGHT: c_int = 0x7D;
pub const RAWKEY_NM_BUTTON_FOURTH: c_int = 0x7E;

/// GPUI modifiers from Amiga `IEQUALIFIER_*` bits. The Amiga/Command keys
/// map to `platform` (like Cmd on macOS / Win on Windows); Ctrl chords stay
/// the primary accelerator since `Keystroke::parse("secondary-…")` resolves
/// to Ctrl off-macOS.
pub fn modifiers_from_qualifier(qualifier: c_int) -> Modifiers {
    Modifiers {
        shift: qualifier & (IEQUALIFIER_LSHIFT | IEQUALIFIER_RSHIFT) != 0,
        control: qualifier & IEQUALIFIER_CONTROL != 0,
        alt: qualifier & (IEQUALIFIER_LALT | IEQUALIFIER_RALT) != 0,
        platform: qualifier & (IEQUALIFIER_LCOMMAND | IEQUALIFIER_RCOMMAND) != 0,
        function: false,
    }
}

/// The NUL-terminated ISO-8859-1 bytes from the C glue as a `String`.
/// Latin-1 maps 1:1 onto the first 256 Unicode scalars, so `u8 as char` is
/// the whole conversion. `None` when empty (unmapped / dead key pending).
pub fn latin1_to_string(bytes: &[u8]) -> Option<String> {
    let len = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    if len == 0 {
        return None;
    }
    Some(bytes[..len].iter().map(|&b| b as char).collect())
}

/// The GPUI keybinding name for a rawkey code: named keys from the (stable
/// Amiga) code table, everything else from the keymap's unmodified
/// translation — so a French layout's `a` binds as "a" even though the code
/// is RAWKEY_Q. Names match the other backends' vocabulary ("enter",
/// "pageup", …) so existing keymaps work unchanged.
pub fn key_name(down_code: c_int, base_chars: &[u8]) -> Option<String> {
    let named = match down_code {
        0x40 => Some("space"),
        0x41 => Some("backspace"),
        0x42 => Some("tab"),
        0x43 | 0x44 => Some("enter"), // keypad enter / return
        0x45 => Some("escape"),
        0x46 => Some("delete"),
        0x47 => Some("insert"),
        0x48 => Some("pageup"),
        0x49 => Some("pagedown"),
        0x4A => None, // keypad minus — fall through to the keymap chars
        0x4B => Some("f11"),
        0x4C => Some("up"),
        0x4D => Some("down"),
        0x4E => Some("right"),
        0x4F => Some("left"),
        0x50 => Some("f1"),
        0x51 => Some("f2"),
        0x52 => Some("f3"),
        0x53 => Some("f4"),
        0x54 => Some("f5"),
        0x55 => Some("f6"),
        0x56 => Some("f7"),
        0x57 => Some("f8"),
        0x58 => Some("f9"),
        0x59 => Some("f10"),
        0x5F => Some("help"),
        0x6F => Some("f12"),
        _ => None,
    };
    if let Some(name) = named {
        return Some(name.to_string());
    }
    // Printables: the unmodified keymap translation, lowercased so the
    // capslock state can't change the binding identity.
    let base = latin1_to_string(base_chars)?;
    let trimmed: String = base.chars().filter(|c| !c.is_control()).collect();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_lowercase())
}
