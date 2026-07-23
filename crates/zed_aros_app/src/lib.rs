//! Minimal AROS entry point: boot editor-core in a gpui window on the AROS
//! platform backend. networking, wasm, and terminal are stubbed in the
//! vendored crates, so this stands up only the pieces an editor needs:
//! settings, base theme, Zed's real keymap, then an `Editor` over a buffer.
//! With a path argument (`ZedAros work:foo.rs`) it opens that file; otherwise
//! it shows a welcome buffer.
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
use settings::{KeymapFile, KeymapFileLoadResult};

const WELCOME: &str = "\
Welcome to Zed on AROS.

editor-core is booting on aarch64-darwin (hosted AROS).
networking, wasm, and terminal are stubbed in this build.

Pass a path to open a file:  ZedAros work:hello.rs
";

// Zed's real macOS keymap (the host is a Mac, so events carry cmd). Bindings
// whose actions this minimal app never registered are skipped on load.
const DEFAULT_KEYMAP: &str = include_str!("../../../assets/keymaps/default-macos.json");

/// Boot the editor. Called by the C shim; returns a process exit code.
#[no_mangle]
pub extern "C" fn zed_aros_main() -> i32 {
    application().run(|cx: &mut App| {
        settings::init(cx);
        theme_settings::init(theme::LoadThemes::JustBase, cx);
        release_channel::init(semver::Version::new(0, 0, 0), cx);
        editor::init(cx);

        // Load Zed's real keymap (not a hand-picked subset). The loader binds
        // every binding whose action is registered and reports the rest as
        // errors, so unregistered app-level actions simply drop out.
        let bindings = match KeymapFile::load(DEFAULT_KEYMAP, cx) {
            KeymapFileLoadResult::Success { key_bindings }
            | KeymapFileLoadResult::SomeFailedToLoad { key_bindings, .. } => key_bindings,
            KeymapFileLoadResult::JsonParseFailure { error } => {
                log::error!("default keymap failed to parse: {error}");
                Vec::new()
            }
        };
        cx.bind_keys(bindings);

        // A path argument opens that file; otherwise show the welcome buffer.
        let (text, title) = match std::env::args().nth(1) {
            Some(path) => match std::fs::read_to_string(&path) {
                Ok(contents) => (contents, path),
                Err(error) => (format!("could not open {path}: {error}\n"), path),
            },
            None => (WELCOME.to_string(), "Zed on AROS".to_string()),
        };

        let bounds = Bounds::centered(None, size(px(900.0), px(600.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |window, cx| {
                cx.new(|cx| {
                    let buffer = cx.new(|cx| Buffer::local(text, cx));
                    let buffer = cx.new(|cx| MultiBuffer::singleton(buffer, cx));
                    Editor::new(EditorMode::full(), buffer, None, window, cx)
                })
            },
        )
        .expect("open editor window");
        let _ = title;
        cx.activate(true);
    });
    0
}
