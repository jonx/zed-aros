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

use std::path::PathBuf;
use std::sync::Arc;

use gpui::{App, AppContext, Window};
use gpui_platform::application;
use settings::{KeymapFile, KeymapFileLoadResult};
use workspace::{AppState, OpenMode, Workspace};

const DEFAULT_KEYMAP: &str = include_str!("../../../assets/keymaps/default-macos.json");

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
        build_window_options: |_, _| Default::default(),
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
}

/// Boot the editor. Called by the C shim; returns a process exit code.
#[no_mangle]
pub extern "C" fn zed_aros_main() -> i32 {
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

        // Map syntax-highlight captures to theme colors; without this the
        // grammars load but every token renders in the default foreground.
        app_state.languages.set_theme(theme::ActiveTheme::theme(cx).clone());

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

/// Populate the workspace status bar (called once the workspace is built).
fn add_status_bar_items(
    workspace: &mut Workspace,
    window: &mut Window,
    cx: &mut gpui::Context<Workspace>,
) {
    let cursor_position = cx.new(|_| go_to_line::cursor_position::CursorPosition::new(workspace));
    workspace.status_bar().update(cx, |status_bar, cx| {
        status_bar.add_right_item(cursor_position, window, cx);
    });
}
