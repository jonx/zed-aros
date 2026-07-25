//! Minimal AROS entry point: boot a Zed `Workspace` in a gpui window on the
//! AROS platform backend. networking/wasm/terminal are stubbed in the vendored
//! crates, so this stands up the workspace with a fake HTTP client and an
//! in-memory database. With a path argument (`ZedAros work:foo.rs`) it opens
//! that file in the workspace (tabs + syntax highlighting + status bar).
//!
//! The runnable AROS binary is produced by hosted/zed/build.sh, which links
//! this staticlib with collect-aros (startup.o + the std pal glue), and calls
//! `zed_aros_main` from a small C shim (hosted/zed/zed_aros_main.c).

mod getrandom_aros;
mod lsp_adapter;

use std::path::PathBuf;
use std::sync::Arc;


use gpui::{App, AppContext, Bounds, WindowBounds, WindowOptions, Window, px, size};
use gpui_platform::application;
use uuid::Uuid;
use settings::{KeymapFile, KeymapFileLoadResult};

/// Window options for the workspace: an explicit centered size that fits the
/// AROS screen, so the whole layout (including the bottom status bar) is on
/// screen rather than clipped by a screen-sized default window.
fn build_window_options(_: Option<Uuid>, cx: &mut App) -> WindowOptions {
    let bounds = Bounds::centered(None, size(px(760.0), px(520.0)), cx);
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        ..Default::default()
    }
}
use workspace::{AppState, OpenMode, Workspace};

const DEFAULT_KEYMAP: &str = include_str!("../../../assets/keymaps/default-macos.json");

gpui::actions!(zed_aros, [Quit]);

/// A minimal app menu built from actions in the crates we link (Zed's own
/// `app_menus` pulls terminal/collab/diagnostics crates we don't). gpui_aros
/// renders these as native Intuition menus (right-mouse-button).
fn app_menus() -> Vec<gpui::Menu> {
    use editor::actions::{Copy, Cut, Paste, Redo, SelectAll, Undo};
    use gpui::{Menu, MenuItem};
    vec![
        Menu {
            name: "Zed on AROS".into(),
            disabled: false,
            items: vec![MenuItem::action("Quit", Quit)],
        },
        Menu {
            name: "File".into(),
            disabled: false,
            items: vec![
                MenuItem::action("New", workspace::NewFile),
                MenuItem::action(
                    "Save",
                    workspace::Save {
                        save_intent: Some(workspace::SaveIntent::Save),
                    },
                ),
            ],
        },
        Menu {
            name: "Edit".into(),
            disabled: false,
            items: vec![
                MenuItem::action("Undo", Undo),
                MenuItem::action("Redo", Redo),
                MenuItem::separator(),
                MenuItem::action("Cut", Cut),
                MenuItem::action("Copy", Copy),
                MenuItem::action("Paste", Paste),
                MenuItem::separator(),
                MenuItem::action("Select All", SelectAll),
            ],
        },
    ]
}

/// Build a minimal AppState: real filesystem + real language registry, but a
/// fake HTTP client (networking is stubbed) and an unavailable node runtime.
fn build_app_state(cx: &mut App) -> Arc<AppState> {
    use client::{Client, UserStore};
    use language::LanguageRegistry;
    use node_runtime::NodeRuntime;

    let fs = Arc::new(fs::RealFs::new(None, cx.background_executor().clone()));
    <dyn fs::Fs>::set_global(fs.clone(), cx);

    let languages = Arc::new(LanguageRegistry::new(cx.background_executor().clone()));
    let node_runtime = NodeRuntime::unavailable();
    register_languages(&languages);

    let clock = Arc::new(clock::RealSystemClock);
    let http = http_client::FakeHttpClient::with_404_response();
    let client = Client::new(clock, http, cx);
    Client::set_global(client.clone(), cx);

    let user_store = cx.new(|cx| UserStore::new(client.clone(), cx));
    let workspace_store = cx.new(|cx| workspace::WorkspaceStore::new(client.clone(), cx));
    let session = cx.new(|cx| session::AppSession::new(session::Session::test(), cx));

    client::init(&client, cx);
    project::Project::init(&client, cx);

    Arc::new(AppState {
        languages,
        client,
        user_store,
        workspace_store,
        fs,
        build_window_options,
        node_runtime,
        session,
    })
}

/// Register every bundled language that has a config, straight from the
/// `grammars` crate (grammar + config + queries). No LSP adapters -- highlighting
/// only, which is all we can use while LSP is OS-gated.
fn register_languages(registry: &Arc<language::LanguageRegistry>) {
    let native = grammars::native_grammars();
    let names: Vec<&'static str> = native.iter().map(|(name, _)| *name).collect();
    registry.register_native_grammars(native);

    for name in names {
        if grammars::get_file(&format!("{name}/config.toml")).is_none() {
            continue;
        }
        let config = grammars::load_config(name);
        registry.register_language(
            config.name.clone(),
            config.grammar.clone(),
            config.matcher.clone(),
            config.hidden,
            None,
            Arc::new(move || {
                Ok(language::LoadedLanguage {
                    config: grammars::load_config(name),
                    queries: grammars::load_queries(name),
                    context_provider: None,
                    toolchain_provider: None,
                    manifest_name: None,
                })
            }),
        );
    }

    // Register a Rust language server that connects over TCP to the host bridge
    // (rust-analyzer), so opening a .rs file gets real diagnostics/completions.
    registry.register_lsp_adapter(
        language::LanguageName::new_static("Rust"),
        Arc::new(lsp_adapter::ArosRustLspAdapter),
    );
}

/// Hidden async-reactor smoke test (`ZedAros --nettest`): connect to a host TCP
/// echo server on 127.0.0.1:9977 via async-io and round-trip a line. Exercises
/// the unified-fd shim + polling Phase B on a real socket.
fn run_nettest() -> i32 {
    use futures_lite::{AsyncReadExt, AsyncWriteExt};
    use std::net::{Ipv4Addr, SocketAddr, TcpStream};

    let addr = SocketAddr::from((Ipv4Addr::new(127, 0, 0, 1), 9977));
    let result: std::io::Result<Vec<u8>> = async_io::block_on(async move {
        let mut stream = async_io::Async::<TcpStream>::connect(addr).await?;
        stream.write_all(b"ping\n").await?;
        let mut buf = [0u8; 64];
        let n = stream.read(&mut buf).await?;
        Ok(buf[..n].to_vec())
    });
    match result {
        Ok(v) if v.starts_with(b"ping") => {
            println!("NETTEST PASS: echoed {} bytes", v.len());
            0
        }
        Ok(v) => {
            println!("NETTEST FAIL: got {:?}", String::from_utf8_lossy(&v));
            1
        }
        Err(e) => {
            println!("NETTEST ERR: {e}");
            2
        }
    }
}

/// Boot the editor. Called by the C shim; returns a process exit code.
#[no_mangle]
pub extern "C" fn zed_aros_main() -> i32 {
    if std::env::args().nth(1).as_deref() == Some("--nettest") {
        return run_nettest();
    }

    // In-memory workspace/kvp database: no data-dir on AROS.
    unsafe { std::env::set_var("ZED_STATELESS", "1") };

    application().run(|cx: &mut App| {
        cx.set_global(db::AppDatabase::new());

        settings::init(cx);
        theme_settings::init(theme::LoadThemes::JustBase, cx);
        release_channel::init(semver::Version::new(0, 0, 0), cx);

        let app_state = build_app_state(cx);
        AppState::set_global(app_state.clone(), cx);

        editor::init(cx);
        workspace::init(app_state.clone(), cx);
        go_to_line::init(cx);
        project_panel::init(cx);

        // Map syntax-highlight captures to theme colors; without this the
        // grammars load but every token renders in the default foreground.
        app_state.languages.set_theme(theme::ActiveTheme::theme(cx).clone());

        // Native Intuition menus (right-mouse-button on AROS).
        cx.on_action(|_: &Quit, cx: &mut App| cx.quit());
        cx.set_menus(app_menus());

        // Zed's real macOS keymap; bindings for unregistered actions are skipped.
        let bindings = match KeymapFile::load(DEFAULT_KEYMAP, cx) {
            KeymapFileLoadResult::Success { key_bindings }
            | KeymapFileLoadResult::SomeFailedToLoad { key_bindings, .. } => key_bindings,
            KeymapFileLoadResult::JsonParseFailure { error } => {
                log::error!("default keymap failed to parse: {error}");
                Vec::new()
            }
        };
        cx.bind_keys(bindings);

        // Open the workspace on the file argument (if any).
        let paths: Vec<PathBuf> = std::env::args().nth(1).map(PathBuf::from).into_iter().collect();
        let task = Workspace::new_local(
            paths,
            app_state.clone(),
            None,
            None,
            Some(Box::new(add_status_bar_items)),
            OpenMode::NewWindow,
            cx,
        );
        task.detach();
        cx.activate(true);
    });
    0
}

/// A minimal always-visible status-bar item (a fixed label). Doubles as a probe
/// that the status bar renders at all, independent of CursorPosition's state.
struct ArosMarker;

impl gpui::Render for ArosMarker {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl gpui::IntoElement {
        use gpui::{ParentElement, Styled};
        gpui::div()
            .px_2()
            .child("Zed on AROS")
            .text_color(gpui::white())
    }
}

impl workspace::StatusItemView for ArosMarker {
    fn set_active_pane_item(
        &mut self,
        _: Option<&dyn workspace::item::ItemHandle>,
        _: &mut Window,
        _: &mut gpui::Context<Self>,
    ) {
    }

    fn hide_setting(&self, _: &App) -> Option<workspace::HideStatusItem> {
        None
    }
}

/// Populate the workspace status bar (called once the workspace is built).
fn add_status_bar_items(
    workspace: &mut Workspace,
    window: &mut Window,
    cx: &mut gpui::Context<Workspace>,
) {
    let marker = cx.new(|_| ArosMarker);
    let cursor_position = cx.new(|_| go_to_line::cursor_position::CursorPosition::new(workspace));
    workspace.status_bar().update(cx, |status_bar, cx| {
        status_bar.add_left_item(marker, window, cx);
        status_bar.add_right_item(cursor_position, window, cx);
    });

    // Load the file tree (project panel) into the left dock, then open the dock.
    cx.spawn_in(window, async move |workspace_handle, cx| {
        if let Ok(panel) =
            project_panel::ProjectPanel::load(workspace_handle.clone(), cx.clone()).await
        {
            workspace_handle
                .update_in(cx, |workspace, window, cx| {
                    workspace.add_panel(panel, window, cx);
                    workspace.focus_panel::<project_panel::ProjectPanel>(window, cx);
                })
                .ok();
        }
    })
    .detach();
}
