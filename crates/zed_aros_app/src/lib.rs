//! Minimal AROS entry point: boot editor-core in a gpui window on the AROS
//! platform backend. networking, wasm, and terminal are stubbed in the
//! vendored crates, so this stands up only the pieces an editor needs:
//! settings, base theme, then an `Editor` view over a singleton buffer.
//!
//! The runnable AROS binary is produced by hosted/zed/build.sh, which links
//! this staticlib with collect-aros (startup.o + the std pal glue), and calls
//! `zed_aros_main` from a small C shim (hosted/zed/zed_aros_main.c).

mod getrandom_aros;

use editor::actions::{
    Backspace, Copy, Cut, Delete, MoveLeft, MoveRight, Newline, Paste, SelectAll, SelectDown,
    SelectLeft, SelectRight, SelectUp, Tab,
};
use editor::{Editor, EditorMode};
use gpui::{App, AppContext, Bounds, KeyBinding, WindowBounds, WindowOptions, px, size};
use zed_actions::editor::{MoveDown, MoveUp};
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

        // The minimal boot loads no keymap, so gpui commits printable text
        // through the input handler but editing actions (backspace, enter,
        // arrows, selection, clipboard) have no bindings. Bind the core set so
        // the buffer is actually editable. Scoped to the Editor context.
        let ctx = Some("Editor");
        cx.bind_keys([
            KeyBinding::new("backspace", Backspace, ctx),
            KeyBinding::new("delete", Delete, ctx),
            KeyBinding::new("enter", Newline, ctx),
            KeyBinding::new("tab", Tab, ctx),
            KeyBinding::new("left", MoveLeft, ctx),
            KeyBinding::new("right", MoveRight, ctx),
            KeyBinding::new("up", MoveUp, ctx),
            KeyBinding::new("down", MoveDown, ctx),
            KeyBinding::new("shift-left", SelectLeft, ctx),
            KeyBinding::new("shift-right", SelectRight, ctx),
            KeyBinding::new("shift-up", SelectUp, ctx),
            KeyBinding::new("shift-down", SelectDown, ctx),
            KeyBinding::new("cmd-a", SelectAll, ctx),
            KeyBinding::new("cmd-c", Copy, ctx),
            KeyBinding::new("cmd-x", Cut, ctx),
            KeyBinding::new("cmd-v", Paste, ctx),
        ]);

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
