//! An AROS Intuition window, wrapping the C glue handle.

use std::cell::RefCell;
use std::ffi::CString;
use std::os::raw::{c_int, c_void};
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AnyWindowHandle, Bounds, Capslock, DevicePixels, DispatchEventResult, GpuSpecs, KeyDownEvent,
    KeyUpEvent, Keystroke, Modifiers, ModifiersChangedEvent, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, PlatformAtlas, PlatformDisplay, PlatformInput,
    PlatformInputHandler, PlatformWindow, Point, PromptButton, PromptLevel, RequestFrameOptions,
    Scene, ScrollDelta, ScrollWheelEvent, Size, TouchPhase, WindowAppearance,
    WindowBackgroundAppearance, WindowBounds, WindowControlArea, WindowParams, px,
};
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, WindowHandle,
};

use crate::glue::{self, GpaEvent};
use crate::renderer::CpuRenderer;

#[derive(Default)]
struct ArosWindowCallbacks {
    request_frame: Option<Box<dyn FnMut(RequestFrameOptions)>>,
    input: Option<Box<dyn FnMut(PlatformInput) -> DispatchEventResult>>,
    active_status_change: Option<Box<dyn FnMut(bool)>>,
    hover_status_change: Option<Box<dyn FnMut(bool)>>,
    resize: Option<Box<dyn FnMut(Size<Pixels>, f32)>>,
    moved: Option<Box<dyn FnMut()>>,
    should_close: Option<Box<dyn FnMut() -> bool>>,
    close: Option<Box<dyn FnOnce()>>,
    appearance_changed: Option<Box<dyn FnMut()>>,
    hit_test_window_control: Option<Box<dyn FnMut() -> Option<WindowControlArea>>>,
}

struct ArosWindowState {
    handle: *mut c_void,
    renderer: CpuRenderer,
    bounds: Bounds<Pixels>,
    title: String,
    input_handler: Option<PlatformInputHandler>,
    is_active: bool,
    is_hovered: bool,
    mouse_position: Point<Pixels>,
    modifiers: Modifiers,
    capslock: Capslock,
    pressed_button: Option<MouseButton>,
    closed: bool,
}

pub(crate) struct ArosWindowInner {
    state: RefCell<ArosWindowState>,
    callbacks: RefCell<ArosWindowCallbacks>,
    display: Rc<dyn PlatformDisplay>,
    atlas: Arc<dyn PlatformAtlas>,
}

impl Drop for ArosWindowInner {
    fn drop(&mut self) {
        let handle = self.state.borrow().handle;
        if !handle.is_null() {
            // SAFETY: handle came from gpa_open_window and is closed once.
            unsafe { glue::gpa_close_window(handle) };
        }
    }
}

pub struct ArosWindow {
    inner: Rc<ArosWindowInner>,
    #[allow(dead_code)]
    handle: AnyWindowHandle,
}

impl ArosWindow {
    pub(crate) fn open(
        handle: AnyWindowHandle,
        params: WindowParams,
        display: Rc<dyn PlatformDisplay>,
    ) -> anyhow::Result<Self> {
        let bounds = params.bounds;
        let width = f32::from(bounds.size.width).max(1.0) as i32;
        let height = f32::from(bounds.size.height).max(1.0) as i32;
        let x = f32::from(bounds.origin.x) as i32;
        let y = f32::from(bounds.origin.y) as i32;

        let title = params
            .titlebar
            .as_ref()
            .and_then(|t| t.title.as_ref())
            .map(|t| t.to_string())
            .unwrap_or_default();
        let c_title = CString::new(title.clone()).unwrap_or_default();

        // SAFETY: FFI to the glue; returns null on failure.
        let raw = unsafe { glue::gpa_open_window(x, y, width, height, c_title.as_ptr()) };
        if raw.is_null() {
            anyhow::bail!("gpa_open_window failed");
        }

        let device_size = Size {
            width: DevicePixels(width),
            height: DevicePixels(height),
        };
        let renderer = CpuRenderer::new(device_size);
        let atlas: Arc<dyn PlatformAtlas> = renderer.sprite_atlas();

        let state = ArosWindowState {
            handle: raw,
            renderer,
            bounds,
            title,
            input_handler: None,
            is_active: true,
            is_hovered: false,
            mouse_position: Point::default(),
            modifiers: Modifiers::default(),
            capslock: Capslock::default(),
            pressed_button: None,
            closed: false,
        };

        let inner = Rc::new(ArosWindowInner {
            state: RefCell::new(state),
            callbacks: RefCell::new(ArosWindowCallbacks::default()),
            display,
            atlas,
        });

        Ok(Self { inner, handle })
    }

    pub(crate) fn inner(&self) -> Rc<ArosWindowInner> {
        self.inner.clone()
    }
}

impl ArosWindowInner {
    /// Union signal mask for this window's message port.
    pub(crate) fn sigmask(&self) -> u32 {
        let handle = self.state.borrow().handle;
        if handle.is_null() {
            return 0;
        }
        // SAFETY: valid handle.
        unsafe { glue::gpa_window_sigmask(handle) }
    }

    /// Fire the stored request-frame callback (GPUI draws the scene within it).
    pub(crate) fn request_frame(&self) {
        if self.state.borrow().closed {
            return;
        }
        let taken = self.callbacks.borrow_mut().request_frame.take();
        if let Some(mut f) = taken {
            f(RequestFrameOptions::default());
            self.callbacks.borrow_mut().request_frame = Some(f);
        }
    }

    /// Drain queued Intuition events and dispatch to the stored callbacks.
    pub(crate) fn poll_events(&self) {
        let handle = self.state.borrow().handle;
        if handle.is_null() {
            return;
        }
        let mut event = GpaEvent::default();
        // SAFETY: valid handle; gpa_poll_event returns 1 while events remain.
        while unsafe { glue::gpa_poll_event(handle, &mut event) } == 1 {
            self.dispatch_event(&event);
            if self.state.borrow().closed {
                return;
            }
        }
    }

    fn dispatch_event(&self, event: &GpaEvent) {
        match event.kind {
            glue::GPA_EVENT_CLOSE => self.handle_close(),
            glue::GPA_EVENT_NEWSIZE => self.handle_resize(),
            glue::GPA_EVENT_REFRESH => self.request_frame(),
            glue::GPA_EVENT_MOUSEMOVE => {
                let position = self.set_mouse_position(event.x, event.y);
                let modifiers = self.state.borrow().modifiers;
                let pressed_button = self.state.borrow().pressed_button;
                self.fire_input(PlatformInput::MouseMove(MouseMoveEvent {
                    position,
                    pressed_button,
                    modifiers,
                }));
            }
            glue::GPA_EVENT_MOUSEDOWN => {
                let position = self.set_mouse_position(event.x, event.y);
                let button = map_button(event.code);
                let modifiers = self.state.borrow().modifiers;
                self.state.borrow_mut().pressed_button = Some(button);
                self.fire_input(PlatformInput::MouseDown(MouseDownEvent {
                    button,
                    position,
                    modifiers,
                    click_count: 1,
                    first_mouse: false,
                }));
            }
            glue::GPA_EVENT_MOUSEUP => {
                let position = self.set_mouse_position(event.x, event.y);
                let button = map_button(event.code);
                let modifiers = self.state.borrow().modifiers;
                self.state.borrow_mut().pressed_button = None;
                self.fire_input(PlatformInput::MouseUp(MouseUpEvent {
                    button,
                    position,
                    modifiers,
                    click_count: 1,
                }));
            }
            glue::GPA_EVENT_RAWKEY => self.handle_rawkey(event),
            _ => {}
        }
    }

    /// Translate an Intuition RAWKEY event: update the modifier/capslock
    /// state from the message qualifiers, route NewMouse wheel codes to
    /// ScrollWheel, and emit KeyDown/KeyUp with a [`Keystroke`] built from
    /// the keymap.library translation the C glue already performed
    /// (`base_chars` = unmodified key for the binding name, `chars` = the
    /// typed characters).
    fn handle_rawkey(&self, event: &GpaEvent) {
        let is_up = event.code & glue::IECODE_UP_PREFIX != 0;
        let down_code = event.code & !glue::IECODE_UP_PREFIX;
        let qualifier = event.qualifier;

        // Qualifiers are authoritative per message — derive the full
        // modifier state from them and report transitions, exactly once.
        let modifiers = modifiers_from_qualifier(qualifier);
        let capslock = Capslock {
            on: qualifier & glue::IEQUALIFIER_CAPSLOCK != 0,
        };
        let (changed, position) = {
            let mut state = self.state.borrow_mut();
            let changed = state.modifiers != modifiers || state.capslock != capslock;
            state.modifiers = modifiers;
            state.capslock = capslock;
            (changed, state.mouse_position)
        };
        if changed {
            self.fire_input(PlatformInput::ModifiersChanged(ModifiersChangedEvent {
                modifiers,
                capslock,
            }));
        }

        // NewMouse standard: the scroll wheel rides the rawkey stream.
        // Wheel-away-from-user scrolls up = positive y lines (the gpui
        // convention shared by the Windows/Linux backends); left mirrors
        // that as positive x. Releases of these codes are noise.
        if (glue::RAWKEY_NM_WHEEL_UP..=glue::RAWKEY_NM_BUTTON_FOURTH).contains(&down_code) {
            let lines = match down_code {
                glue::RAWKEY_NM_WHEEL_UP => Point { x: 0.0, y: 1.0 },
                glue::RAWKEY_NM_WHEEL_DOWN => Point { x: 0.0, y: -1.0 },
                glue::RAWKEY_NM_WHEEL_LEFT => Point { x: 1.0, y: 0.0 },
                glue::RAWKEY_NM_WHEEL_RIGHT => Point { x: -1.0, y: 0.0 },
                _ => return, // fourth button etc. — not a wheel, not a key
            };
            if !is_up {
                self.fire_input(PlatformInput::ScrollWheel(ScrollWheelEvent {
                    position,
                    delta: ScrollDelta::Lines(lines),
                    modifiers,
                    touch_phase: TouchPhase::Moved,
                }));
            }
            return;
        }

        // The modifier keys themselves only feed the state above.
        if (glue::RAWKEY_MODIFIER_FIRST..=glue::RAWKEY_MODIFIER_LAST).contains(&down_code) {
            return;
        }

        let Some(key) = key_name(down_code, &event.base_chars) else {
            // Unmapped code (or a dead key waiting for its successor) —
            // nothing sensible to report.
            return;
        };

        // The typed characters, when they're actual text: control/platform
        // chords produce control bytes (Ctrl-A = 0x01) or shortcuts, not
        // input — same suppression the other backends apply.
        let key_char = if modifiers.control || modifiers.platform {
            None
        } else {
            latin1_to_string(&event.chars).filter(|s| !s.chars().any(char::is_control))
        };

        let keystroke = Keystroke {
            modifiers,
            key,
            key_char,
        };
        if is_up {
            self.fire_input(PlatformInput::KeyUp(KeyUpEvent { keystroke }));
        } else {
            self.fire_input(PlatformInput::KeyDown(KeyDownEvent {
                keystroke,
                is_held: qualifier & glue::IEQUALIFIER_REPEAT != 0,
                prefer_character_input: false,
            }));
        }
    }

    fn set_mouse_position(&self, x: c_int, y: c_int) -> Point<Pixels> {
        let position = Point {
            x: px(x as f32),
            y: px(y as f32),
        };
        self.state.borrow_mut().mouse_position = position;
        position
    }

    fn fire_input(&self, input: PlatformInput) {
        let taken = self.callbacks.borrow_mut().input.take();
        if let Some(mut f) = taken {
            let _ = f(input);
            self.callbacks.borrow_mut().input = Some(f);
        }
    }

    fn handle_resize(&self) {
        let handle = self.state.borrow().handle;
        let (mut w, mut h) = (0i32, 0i32);
        // SAFETY: valid handle; writes both out-params.
        unsafe { glue::gpa_inner_size(handle, &mut w, &mut h) };
        let (w, h) = (w.max(1), h.max(1));
        let size = Size {
            width: px(w as f32),
            height: px(h as f32),
        };
        {
            let mut state = self.state.borrow_mut();
            state.bounds.size = size;
            state.renderer.update_drawable_size(Size {
                width: DevicePixels(w),
                height: DevicePixels(h),
            });
        }
        let taken = self.callbacks.borrow_mut().resize.take();
        if let Some(mut f) = taken {
            f(size, 1.0);
            self.callbacks.borrow_mut().resize = Some(f);
        }
    }

    fn handle_close(&self) {
        // Ask GPUI whether to close (default: yes).
        let should_close = {
            let taken = self.callbacks.borrow_mut().should_close.take();
            if let Some(mut f) = taken {
                let result = f();
                self.callbacks.borrow_mut().should_close = Some(f);
                result
            } else {
                true
            }
        };
        if !should_close {
            return;
        }
        self.state.borrow_mut().closed = true;
        let close = self.callbacks.borrow_mut().close.take();
        if let Some(f) = close {
            f();
        }
    }
}

fn map_button(code: c_int) -> MouseButton {
    match code {
        glue::GPA_BUTTON_RIGHT => MouseButton::Right,
        glue::GPA_BUTTON_MIDDLE => MouseButton::Middle,
        glue::GPA_BUTTON_LEFT | _ => MouseButton::Left,
    }
}

/// GPUI modifiers from Amiga `IEQUALIFIER_*` bits. The Amiga/Command keys
/// map to `platform` (like Cmd on macOS / Win on Windows); Ctrl chords stay
/// the primary accelerator since `Keystroke::parse("secondary-…")` resolves
/// to Ctrl off-macOS.
fn modifiers_from_qualifier(qualifier: c_int) -> Modifiers {
    Modifiers {
        shift: qualifier & (glue::IEQUALIFIER_LSHIFT | glue::IEQUALIFIER_RSHIFT) != 0,
        control: qualifier & glue::IEQUALIFIER_CONTROL != 0,
        alt: qualifier & (glue::IEQUALIFIER_LALT | glue::IEQUALIFIER_RALT) != 0,
        platform: qualifier & (glue::IEQUALIFIER_LCOMMAND | glue::IEQUALIFIER_RCOMMAND) != 0,
        function: false,
    }
}

/// The NUL-terminated ISO-8859-1 bytes from the C glue as a `String`.
/// Latin-1 maps 1:1 onto the first 256 Unicode scalars, so `u8 as char` is
/// the whole conversion. `None` when empty (unmapped / dead key pending).
fn latin1_to_string(bytes: &[u8]) -> Option<String> {
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
fn key_name(down_code: c_int, base_chars: &[u8]) -> Option<String> {
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

impl HasWindowHandle for ArosWindow {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        Err(HandleError::Unavailable)
    }
}

impl HasDisplayHandle for ArosWindow {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        Err(HandleError::Unavailable)
    }
}

impl PlatformWindow for ArosWindow {
    fn bounds(&self) -> Bounds<Pixels> {
        self.inner.state.borrow().bounds
    }

    fn is_maximized(&self) -> bool {
        false
    }

    fn window_bounds(&self) -> WindowBounds {
        WindowBounds::Windowed(self.bounds())
    }

    fn content_size(&self) -> Size<Pixels> {
        self.inner.state.borrow().bounds.size
    }

    fn resize(&mut self, _size: Size<Pixels>) {
        // The user resizes AROS windows via the size gadget; programmatic
        // resize is a no-op (best-effort).
    }

    fn scale_factor(&self) -> f32 {
        1.0
    }

    fn appearance(&self) -> WindowAppearance {
        WindowAppearance::Light
    }

    fn display(&self) -> Option<Rc<dyn PlatformDisplay>> {
        Some(self.inner.display.clone())
    }

    fn mouse_position(&self) -> Point<Pixels> {
        self.inner.state.borrow().mouse_position
    }

    fn modifiers(&self) -> Modifiers {
        self.inner.state.borrow().modifiers
    }

    fn capslock(&self) -> Capslock {
        self.inner.state.borrow().capslock
    }

    fn set_input_handler(&mut self, input_handler: PlatformInputHandler) {
        self.inner.state.borrow_mut().input_handler = Some(input_handler);
    }

    fn take_input_handler(&mut self) -> Option<PlatformInputHandler> {
        self.inner.state.borrow_mut().input_handler.take()
    }

    fn prompt(
        &self,
        _level: PromptLevel,
        _msg: &str,
        _detail: Option<&str>,
        _answers: &[PromptButton],
    ) -> Option<futures::channel::oneshot::Receiver<usize>> {
        None
    }

    fn activate(&self) {
        self.inner.state.borrow_mut().is_active = true;
    }

    fn is_active(&self) -> bool {
        self.inner.state.borrow().is_active
    }

    fn is_hovered(&self) -> bool {
        self.inner.state.borrow().is_hovered
    }

    fn background_appearance(&self) -> WindowBackgroundAppearance {
        WindowBackgroundAppearance::Opaque
    }

    fn set_title(&mut self, title: &str) {
        let handle = self.inner.state.borrow().handle;
        if let Ok(c_title) = CString::new(title) {
            // SAFETY: valid handle; glue copies the string.
            unsafe { glue::gpa_set_title(handle, c_title.as_ptr()) };
        }
        self.inner.state.borrow_mut().title = title.to_owned();
    }

    fn set_background_appearance(&self, _background_appearance: WindowBackgroundAppearance) {}

    fn minimize(&self) {}

    fn zoom(&self) {}

    fn toggle_fullscreen(&self) {}

    fn is_fullscreen(&self) -> bool {
        false
    }

    fn on_request_frame(&self, callback: Box<dyn FnMut(RequestFrameOptions)>) {
        self.inner.callbacks.borrow_mut().request_frame = Some(callback);
    }

    fn on_input(&self, callback: Box<dyn FnMut(PlatformInput) -> DispatchEventResult>) {
        self.inner.callbacks.borrow_mut().input = Some(callback);
    }

    fn on_active_status_change(&self, callback: Box<dyn FnMut(bool)>) {
        self.inner.callbacks.borrow_mut().active_status_change = Some(callback);
    }

    fn on_hover_status_change(&self, callback: Box<dyn FnMut(bool)>) {
        self.inner.callbacks.borrow_mut().hover_status_change = Some(callback);
    }

    fn on_resize(&self, callback: Box<dyn FnMut(Size<Pixels>, f32)>) {
        self.inner.callbacks.borrow_mut().resize = Some(callback);
    }

    fn on_moved(&self, callback: Box<dyn FnMut()>) {
        self.inner.callbacks.borrow_mut().moved = Some(callback);
    }

    fn on_should_close(&self, callback: Box<dyn FnMut() -> bool>) {
        self.inner.callbacks.borrow_mut().should_close = Some(callback);
    }

    fn on_hit_test_window_control(&self, callback: Box<dyn FnMut() -> Option<WindowControlArea>>) {
        self.inner.callbacks.borrow_mut().hit_test_window_control = Some(callback);
    }

    fn on_close(&self, callback: Box<dyn FnOnce()>) {
        self.inner.callbacks.borrow_mut().close = Some(callback);
    }

    fn on_appearance_changed(&self, callback: Box<dyn FnMut()>) {
        self.inner.callbacks.borrow_mut().appearance_changed = Some(callback);
    }

    fn draw(&self, scene: &Scene) {
        let mut state = self.inner.state.borrow_mut();
        if state.closed {
            return;
        }
        state.renderer.draw(scene);
        let width = state.renderer.width() as i32;
        let height = state.renderer.height() as i32;
        let stride = width * 4;
        let handle = state.handle;
        let data = state.renderer.framebuffer();
        // SAFETY: valid handle; data is width*height*4 tightly-rowed RGBA.
        unsafe {
            glue::gpa_blit(
                handle,
                data.as_ptr() as *const c_void,
                stride,
                0,
                0,
                width,
                height,
            );
        }
    }

    fn sprite_atlas(&self) -> Arc<dyn PlatformAtlas> {
        self.inner.atlas.clone()
    }

    fn is_subpixel_rendering_supported(&self) -> bool {
        false
    }

    fn gpu_specs(&self) -> Option<GpuSpecs> {
        Some(GpuSpecs {
            is_software_emulated: true,
            device_name: "AROS CPU".to_owned(),
            driver_name: "tiny-skia".to_owned(),
            driver_info: String::new(),
        })
    }

    fn update_ime_position(&self, _bounds: Bounds<Pixels>) {}
}
