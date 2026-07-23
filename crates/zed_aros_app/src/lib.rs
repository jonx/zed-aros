//! Minimal AROS entry point: boot editor-core in a gpui window on the AROS
//! platform backend. networking, wasm, and terminal are stubbed in the
//! vendored crates, so this stands up only the pieces an editor needs:
//! settings, base theme, then an `Editor` view over a singleton buffer.
//!
//! The runnable AROS binary is produced by hosted/zed/build.sh, which links
//! this staticlib with collect-aros (startup.o + the std pal glue), and calls
//! `zed_aros_main` from a small C shim (hosted/zed/zed_aros_main.c).

mod getrandom_aros;

use editor::{Editor, EditorMode};
use gpui::{App, AppContext, Bounds, WindowBounds, WindowOptions, px, size};
use gpui_platform::application;
use language::Buffer;
use multi_buffer::MultiBuffer;

const WELCOME: &str = "\
Welcome to Zed on AROS.

editor-core is booting on aarch64-darwin (hosted AROS).
networking, wasm, and terminal are stubbed in this build.
";

/// Boot the editor. Called by the C shim; returns a process exit code.
#[no_mangle]
pub extern "C" fn zed_aros_main() -> i32 {
    application().run(|cx: &mut App| {
        settings::init(cx);
        theme_settings::init(theme::LoadThemes::JustBase, cx);
        release_channel::init(semver::Version::new(0, 0, 0), cx);
        editor::init(cx);

        let bounds = Bounds::centered(None, size(px(900.0), px(600.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |window, cx| {
                cx.new(|cx| {
                    let buffer = cx.new(|cx| Buffer::local(WELCOME, cx));
                    let buffer = cx.new(|cx| MultiBuffer::singleton(buffer, cx));
                    Editor::new(EditorMode::full(), buffer, None, window, cx)
                })
            },
        )
        .expect("open editor window");
        cx.activate(true);
    });
    0
}
