//! GPUI-on-AROS smoke: opens a real Intuition window through the gpui_aros
//! backend and renders a scene that exercises every implemented renderer
//! primitive — gradient quad fills, rounded corners + borders, drop shadow,
//! monochrome glyphs (text), and keyboard input (the last keystroke echoes).
//!
//! Linked as a staticlib into an AROS C: command by `link-aros.sh`; the C
//! harness (`c/smoke_main.c`) owns AROS startup and calls
//! [`aros_gpui_smoke_main`]. Drive it under `graft/aros-ctl`:
//!
//! ```sh
//! crates/gpui_aros_smoke/link-aros.sh     # build + link + deploy C:GpuiSmoke
//! graft/aros-ctl run; graft/aros-ctl type "c:gpuismoke"; graft/aros-ctl enter
//! graft/aros-ctl shot smoke.png
//! ```

use gpui::{
    App, Bounds, Context, FocusHandle, KeyDownEvent, SharedString, Window, WindowBounds,
    WindowOptions, div, linear_gradient, linear_color_stop, prelude::*, px, rgb, size,
};

struct Smoke {
    focus_handle: FocusHandle,
    last_key: SharedString,
    presses: usize,
}

impl Smoke {
    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            last_key: "none yet — type!".into(),
            presses: 0,
        }
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.presses += 1;
        self.last_key = format!("{} (#{})", event.keystroke, self.presses).into();
        cx.notify();
    }
}

impl Render for Smoke {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::on_key_down))
            .flex()
            .flex_col()
            .size_full()
            .items_center()
            .justify_center()
            .gap_4()
            // Gradient fill — exercises background_paint's LinearGradient.
            .bg(linear_gradient(
                160.,
                linear_color_stop(rgb(0x1e2a5a), 0.),
                linear_color_stop(rgb(0x0b0f1e), 1.),
            ))
            .text_color(rgb(0xffffff))
            .child(div().text_2xl().child("GPUI on AROS"))
            .child(div().text_sm().text_color(rgb(0x9fb4ff)).child(
                "CPU renderer: quads · gradients · glyphs · shadows · sprites",
            ))
            // Rounded, bordered, shadowed card — quad corners/border + shadow.
            .child(
                div()
                    .flex()
                    .gap_2()
                    .p_3()
                    .rounded_lg()
                    .bg(rgb(0x18213d))
                    .border_1()
                    .border_color(rgb(0x3d5afe))
                    .shadow_lg()
                    .child(div().size_8().rounded_md().bg(rgb(0xe53935)))
                    .child(div().size_8().rounded_md().bg(rgb(0xfb8c00)))
                    .child(div().size_8().rounded_md().bg(rgb(0xfdd835)))
                    .child(div().size_8().rounded_md().bg(rgb(0x43a047)))
                    .child(div().size_8().rounded_md().bg(rgb(0x1e88e5))),
            )
            // Keyboard echo — proves rawkey → Keystroke end to end.
            .child(
                div()
                    .text_sm()
                    .child(format!("last key: {}", self.last_key)),
            )
    }
}

/// Entry point called by the C harness after AROS startup. Returns the magic
/// on clean run-loop exit so the harness can print PASS.
#[unsafe(no_mangle)]
pub extern "C" fn aros_gpui_smoke_main() -> u32 {
    gpui_platform::application().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(560.), px(360.)), cx);
        let window = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |window, cx| {
                let smoke = cx.new(Smoke::new);
                let focus = smoke.read(cx).focus_handle.clone();
                window.focus(&focus, cx);
                smoke
            },
        );
        if window.is_err() {
            cx.quit();
        }
    });
    0x47505541 // "GPUA"
}

/// getrandom v0.3 custom backend (`getrandom_backend="custom"` in the target
/// rustflags): AROS posixc provides a host-backed arc4random_buf CSPRNG.
#[unsafe(no_mangle)]
unsafe extern "Rust" fn __getrandom_v03_custom(
    dest: *mut u8,
    len: usize,
) -> Result<(), getrandom::Error> {
    unsafe extern "C" {
        fn arc4random_buf(buf: *mut core::ffi::c_void, nbytes: usize);
    }
    unsafe { arc4random_buf(dest.cast(), len) };
    Ok(())
}
