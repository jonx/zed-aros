//! Static-library entry point for AROS.
//!
//! `collect-aros` links a staticlib rather than driving the host linker, so the
//! binary crate cannot be used directly. This target compiles the same
//! `main.rs` and exposes a C-callable entry for the AROS startup shim.
//!
//! It is deliberately empty on every other target, so adding this library
//! target costs other platforms nothing.

#![allow(unused_crate_dependencies)]

// The custom getrandom backend must be linked into whichever crate provides the
// AROS entry point; the binary target gets it the same way from zed_aros_app.
#[cfg(target_os = "aros")]
mod getrandom_aros;

#[cfg(target_os = "aros")]
#[path = "main.rs"]
mod zed_main;

// `main.rs` is the crate root when built as the `zed` binary, so its submodules
// refer to its items as `crate::…`. Here it is a module instead, so re-export
// those items at the real root to keep those paths resolving.
#[cfg(target_os = "aros")]
pub(crate) use zed_main::{
    STARTUP_TIME, handle_open_request, restore_or_create_workspace, stdout_is_a_pty, zed,
};

/// Entry point called by the AROS C shim (see hosted/zed/zed_main_aros.c).
#[cfg(target_os = "aros")]
#[unsafe(no_mangle)]
pub extern "C" fn zed_aros_main() -> i32 {
    zed_main::main();
    0
}
