//! The AROS platform: owns the run loop, dispatcher, display and text system.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use futures::channel::oneshot;
use gpui::{
    Action, AnyWindowHandle, BackgroundExecutor, ClipboardItem, CursorStyle, DummyKeyboardMapper,
    ForegroundExecutor, Keymap, Menu, MenuItem, PathPromptOptions, Platform, PlatformDisplay,
    PlatformKeyboardLayout, PlatformKeyboardMapper, PlatformTextSystem, PlatformWindow, Task,
    ThermalState, WindowAppearance, WindowParams,
};

use crate::dispatcher::ArosDispatcher;
use crate::display::ArosDisplay;
use crate::text;
use crate::window::{ArosWindow, ArosWindowInner};

// ~30fps: the run loop parks at most this long between frames.
const FRAME_INTERVAL_MS: i32 = 33;

#[derive(Default)]
struct PlatformCallbacks {
    open_urls: Option<Box<dyn FnMut(Vec<String>)>>,
    quit: Option<Box<dyn FnMut()>>,
    reopen: Option<Box<dyn FnMut()>>,
    app_menu_action: Option<Box<dyn FnMut(&dyn Action)>>,
    will_open_app_menu: Option<Box<dyn FnMut()>>,
    validate_app_menu_command: Option<Box<dyn FnMut(&dyn Action) -> bool>>,
    keyboard_layout_change: Option<Box<dyn FnMut()>>,
    thermal_state_change: Option<Box<dyn FnMut()>>,
}

pub struct ArosPlatform {
    background_executor: BackgroundExecutor,
    foreground_executor: ForegroundExecutor,
    dispatcher: Arc<ArosDispatcher>,
    text_system: Arc<dyn PlatformTextSystem>,
    display: Rc<dyn PlatformDisplay>,
    windows: RefCell<Vec<Weak<ArosWindowInner>>>,
    active_window: RefCell<Option<AnyWindowHandle>>,
    callbacks: RefCell<PlatformCallbacks>,
    quit_requested: Cell<bool>,
    cursor_visible: Cell<bool>,
    #[allow(dead_code)]
    headless: bool,
}

impl ArosPlatform {
    pub fn new(headless: bool) -> Self {
        // Record the main task + allocate the run-loop wake signal before
        // any dispatcher thread exists that could try to Signal() it.
        // SAFETY: FFI; platform creation happens on the main thread.
        unsafe { crate::glue::gpa_init_main() };
        let dispatcher = Arc::new(ArosDispatcher::new());
        let background_executor = BackgroundExecutor::new(dispatcher.clone());
        let foreground_executor = ForegroundExecutor::new(dispatcher.clone());
        let text_system = text::new_text_system();
        let display: Rc<dyn PlatformDisplay> = Rc::new(ArosDisplay::new());

        Self {
            background_executor,
            foreground_executor,
            dispatcher,
            text_system,
            display,
            windows: RefCell::new(Vec::new()),
            active_window: RefCell::new(None),
            callbacks: RefCell::new(PlatformCallbacks::default()),
            quit_requested: Cell::new(false),
            cursor_visible: Cell::new(true),
            headless,
        }
    }

    /// Run every queued main-thread runnable. Runnables may open windows or
    /// request quit, so this holds no other borrow.
    fn drain_main_thread(&self) {
        let queue = self.dispatcher.main_queue();
        loop {
            let runnable = queue.lock().pop_front();
            match runnable {
                Some(runnable) => {
                    runnable.run();
                }
                None => break,
            }
        }
    }

    /// Live windows, with dead weaks pruned.
    fn live_windows(&self) -> Vec<Rc<ArosWindowInner>> {
        let mut windows = self.windows.borrow_mut();
        windows.retain(|w| w.strong_count() > 0);
        windows.iter().filter_map(Weak::upgrade).collect()
    }
}

impl Platform for ArosPlatform {
    fn background_executor(&self) -> BackgroundExecutor {
        self.background_executor.clone()
    }

    fn foreground_executor(&self) -> ForegroundExecutor {
        self.foreground_executor.clone()
    }

    fn text_system(&self) -> Arc<dyn PlatformTextSystem> {
        self.text_system.clone()
    }

    fn run(&self, on_finish_launching: Box<dyn 'static + FnOnce()>) {
        on_finish_launching();

        while !self.quit_requested.get() {
            let frame_start = Instant::now();

            self.drain_main_thread();
            if self.quit_requested.get() {
                break;
            }

            let windows = self.live_windows();
            for window in &windows {
                window.poll_events();
            }
            for window in &windows {
                window.request_frame();
            }
            if self.quit_requested.get() {
                break;
            }

            // Park on the union of window signal masks — plus the dispatcher
            // wake signal, so dispatch_on_main_thread cuts the park short —
            // up to the frame budget.
            let mask = windows.iter().fold(0u32, |acc, w| acc | w.sigmask())
                // SAFETY: FFI; returns 0 before gpa_init_main.
                | unsafe { crate::glue::gpa_wake_sigmask() };
            let elapsed = frame_start.elapsed();
            let budget = Duration::from_millis(FRAME_INTERVAL_MS as u64);
            let remaining = budget.saturating_sub(elapsed).as_millis() as i32;
            if remaining > 0 {
                // SAFETY: FFI; mask==0 degrades to a plain sleep.
                unsafe { crate::glue::gpa_wait_timeout_ms(mask, remaining) };
            }
        }
    }

    fn quit(&self) {
        self.quit_requested.set(true);
    }

    fn restart(&self, _binary_path: Option<PathBuf>) {}

    fn activate(&self, _ignoring_other_apps: bool) {}

    fn hide(&self) {}

    fn hide_other_apps(&self) {}

    fn unhide_other_apps(&self) {}

    fn displays(&self) -> Vec<Rc<dyn PlatformDisplay>> {
        vec![self.display.clone()]
    }

    fn primary_display(&self) -> Option<Rc<dyn PlatformDisplay>> {
        Some(self.display.clone())
    }

    fn active_window(&self) -> Option<AnyWindowHandle> {
        *self.active_window.borrow()
    }

    fn open_window(
        &self,
        handle: AnyWindowHandle,
        options: WindowParams,
    ) -> Result<Box<dyn PlatformWindow>> {
        let window = ArosWindow::open(handle, options, self.display.clone())?;
        self.windows.borrow_mut().push(Rc::downgrade(&window.inner()));
        *self.active_window.borrow_mut() = Some(handle);
        Ok(Box::new(window))
    }

    fn window_appearance(&self) -> WindowAppearance {
        WindowAppearance::Light
    }

    fn open_url(&self, _url: &str) {}

    fn on_open_urls(&self, callback: Box<dyn FnMut(Vec<String>)>) {
        self.callbacks.borrow_mut().open_urls = Some(callback);
    }

    fn register_url_scheme(&self, _url: &str) -> Task<Result<()>> {
        Task::ready(Ok(()))
    }

    fn prompt_for_paths(
        &self,
        _options: PathPromptOptions,
    ) -> oneshot::Receiver<Result<Option<Vec<PathBuf>>>> {
        let (tx, rx) = oneshot::channel();
        tx.send(Ok(None)).ok();
        rx
    }

    fn prompt_for_new_path(
        &self,
        _directory: &Path,
        _suggested_name: Option<&str>,
    ) -> oneshot::Receiver<Result<Option<PathBuf>>> {
        let (tx, rx) = oneshot::channel();
        tx.send(Ok(None)).ok();
        rx
    }

    fn can_select_mixed_files_and_dirs(&self) -> bool {
        false
    }

    fn reveal_path(&self, _path: &Path) {}

    fn open_with_system(&self, _path: &Path) {}

    fn on_quit(&self, callback: Box<dyn FnMut()>) {
        self.callbacks.borrow_mut().quit = Some(callback);
    }

    fn on_reopen(&self, callback: Box<dyn FnMut()>) {
        self.callbacks.borrow_mut().reopen = Some(callback);
    }

    fn set_menus(&self, _menus: Vec<Menu>, _keymap: &Keymap) {}

    fn set_dock_menu(&self, _menu: Vec<MenuItem>, _keymap: &Keymap) {}

    fn on_app_menu_action(&self, callback: Box<dyn FnMut(&dyn Action)>) {
        self.callbacks.borrow_mut().app_menu_action = Some(callback);
    }

    fn on_will_open_app_menu(&self, callback: Box<dyn FnMut()>) {
        self.callbacks.borrow_mut().will_open_app_menu = Some(callback);
    }

    fn on_validate_app_menu_command(&self, callback: Box<dyn FnMut(&dyn Action) -> bool>) {
        self.callbacks.borrow_mut().validate_app_menu_command = Some(callback);
    }

    fn thermal_state(&self) -> ThermalState {
        ThermalState::Nominal
    }

    fn on_thermal_state_change(&self, callback: Box<dyn FnMut()>) {
        self.callbacks.borrow_mut().thermal_state_change = Some(callback);
    }

    fn compositor_name(&self) -> &'static str {
        "AROS"
    }

    fn app_path(&self) -> Result<PathBuf> {
        Ok(std::env::current_exe()?)
    }

    fn path_for_auxiliary_executable(&self, _name: &str) -> Result<PathBuf> {
        Err(anyhow::anyhow!(
            "path_for_auxiliary_executable is not available on AROS"
        ))
    }

    fn set_cursor_style(&self, _style: CursorStyle) {}

    fn hide_cursor_until_mouse_moves(&self) {
        self.cursor_visible.set(false);
    }

    fn is_cursor_visible(&self) -> bool {
        self.cursor_visible.get()
    }

    fn should_auto_hide_scrollbars(&self) -> bool {
        false
    }

    fn read_from_clipboard(&self) -> Option<ClipboardItem> {
        let mut buf: *mut std::ffi::c_void = std::ptr::null_mut();
        let mut len: std::os::raw::c_int = 0;
        // SAFETY: FFI; on success `buf` is an AllocVec'd buffer of `len`
        // bytes that we must hand back to gpa_free.
        unsafe {
            if crate::glue::gpa_clipboard_read_text(&mut buf, &mut len) != 0 || buf.is_null() {
                return None;
            }
            let bytes = std::slice::from_raw_parts(buf as *const u8, len.max(0) as usize);
            // FTXT is system-charset text — ISO-8859-1 on stock AROS, which
            // maps 1:1 onto the first 256 Unicode scalars.
            let text: String = bytes.iter().map(|&b| b as char).collect();
            crate::glue::gpa_free(buf);
            Some(ClipboardItem::new_string(text))
        }
    }

    fn write_to_clipboard(&self, item: ClipboardItem) {
        let Some(text) = item.text() else {
            return; // image-only clips: nothing to publish as FTXT
        };
        // UTF-8 -> ISO-8859-1, lossy: anything past U+00FF becomes '?'
        // (FTXT has no wider charset; a CSET/UTF8 chunk is a v2 nicety).
        let bytes: Vec<u8> = text
            .chars()
            .map(|c| if (c as u32) <= 0xFF { c as u32 as u8 } else { b'?' })
            .collect();
        // SAFETY: FFI; the glue copies the bytes before returning.
        unsafe {
            if crate::glue::gpa_clipboard_write_text(
                bytes.as_ptr() as *const std::ffi::c_void,
                bytes.len() as std::os::raw::c_int,
            ) != 0
            {
                log::warn!("gpui_aros: clipboard write failed");
            }
        }
    }

    fn write_credentials(&self, _url: &str, _username: &str, _password: &[u8]) -> Task<Result<()>> {
        Task::ready(Ok(()))
    }

    fn read_credentials(&self, _url: &str) -> Task<Result<Option<(String, Vec<u8>)>>> {
        Task::ready(Ok(None))
    }

    fn delete_credentials(&self, _url: &str) -> Task<Result<()>> {
        Task::ready(Ok(()))
    }

    fn keyboard_layout(&self) -> Box<dyn PlatformKeyboardLayout> {
        Box::new(ArosKeyboardLayout)
    }

    fn keyboard_mapper(&self) -> Rc<dyn PlatformKeyboardMapper> {
        Rc::new(DummyKeyboardMapper)
    }

    fn on_keyboard_layout_change(&self, callback: Box<dyn FnMut()>) {
        self.callbacks.borrow_mut().keyboard_layout_change = Some(callback);
    }
}

struct ArosKeyboardLayout;

impl PlatformKeyboardLayout for ArosKeyboardLayout {
    fn id(&self) -> &str {
        "us"
    }

    fn name(&self) -> &str {
        "US"
    }
}
