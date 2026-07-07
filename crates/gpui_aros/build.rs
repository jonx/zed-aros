//! Build script for gpui_aros.
//!
//! The C glue (`c/gpui_aros_glue.c`) wraps AROS Intuition/CyberGraphics. It is
//! only compiled when targeting AROS *and* the AROS SDK proto headers are
//! present. Those headers live in the canonical build tree (stable location
//! `$AROS_BUILD`, default `~/aros-build`; the old `/tmp/arosbuild` copy gets
//! eaten by the macOS periodic /tmp cleaner). When they are absent we skip the
//! cc invocation so that a plain `cargo check` still succeeds — the final
//! runnable AROS binary links the glue object via collect-aros at link time
//! (like hosted/feraille).
use std::env;
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=c/gpui_aros_glue.c");

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os != "aros" {
        return;
    }

    // The headers the glue needs, produced by the AROS OS build. Probe every
    // include root the glue reaches (proto/, exec/, devices/, …) — the /tmp
    // tree gets GC'd piecemeal, so a single surviving file (it happens:
    // proto/exec.h outlived exec/types.h once) must not trick us into a
    // doomed cc invocation.
    let aros_build = env::var("AROS_BUILD").unwrap_or_else(|_| {
        format!("{}/aros-build", env::var("HOME").unwrap_or_default())
    });
    let include_root = format!("{aros_build}/bin/darwin-aarch64/gen/include");
    let include_root = include_root.as_str();
    let probes = [
        "proto/exec.h",
        "proto/intuition.h",
        "proto/cybergraphics.h",
        "proto/keymap.h",
        "exec/types.h",
        "devices/inputevent.h",
        "intuition/intuition.h",
        "cybergraphx/cybergraphics.h",
    ];
    if let Some(missing) = probes
        .iter()
        .find(|p| !Path::new(include_root).join(p).exists())
    {
        println!(
            "cargo:warning=gpui_aros glue deferred to link time (SDK header {missing} absent)"
        );
        return;
    }

    // CC + the non-path CFLAGS (host clang, ELF triple, large code model, x18
    // reserved) come from the workspace .cargo/config.toml. The machine-specific
    // pieces are set here so the tracked config stays path-free: the SDK include
    // roots (from $AROS_BUILD, computed above) and the llvm-ar archiver (Apple ar
    // silently makes empty archives from ELF objects) from $AROS_CROSSTOOLS
    // (default ~/aros-crosstools). -Wno-pointer-sign matches the AROS C
    // convention (STRPTR is unsigned char*; string literals are plain char*).
    let aros_crosstools = env::var("AROS_CROSSTOOLS").unwrap_or_else(|_| {
        format!("{}/aros-crosstools", env::var("HOME").unwrap_or_default())
    });
    cc::Build::new()
        .file("c/gpui_aros_glue.c")
        .archiver(format!("{aros_crosstools}/bin/llvm-ar"))
        .flag("-Wno-pointer-sign")
        .include(include_root)
        .include(format!("{include_root}/aros/posixc"))
        .include(format!("{include_root}/aros/stdc"))
        .include(format!("{aros_build}/bin/darwin-aarch64/AROS/Developer/include"))
        .compile("gpui_aros_glue");
}
