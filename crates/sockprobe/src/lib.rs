//! Which tokio runtime configurations can be built on AROS?
//!
//! zed's gpui_tokio::init uses new_multi_thread().enable_all(), which turns on
//! the I/O driver (mio). If that cannot start here, this reports exactly which
//! parts work so the AROS arm can enable only those.

fn try_build(label: &str, f: impl FnOnce() -> std::io::Result<tokio::runtime::Runtime>) {
    match f() {
        Ok(rt) => {
            // prove it actually runs a task
            let v = rt.block_on(async { 41 + 1 });
            println!("[TOKIO] {label}: OK (block_on -> {v})");
        }
        Err(e) => println!("[TOKIO] {label}: ERR {e:?}"),
    }
}

#[no_mangle]
pub extern "C" fn sockprobe_main() -> i32 {
    use tokio::runtime::Builder;

    try_build("multi_thread bare", || {
        Builder::new_multi_thread().worker_threads(2).build()
    });
    try_build("multi_thread + enable_time", || {
        Builder::new_multi_thread().worker_threads(2).enable_time().build()
    });
    try_build("current_thread + enable_time", || {
        Builder::new_current_thread().enable_time().build()
    });
    // Last: this is what gpui_tokio uses today, and it aborts (mio has no
    // os-poll backend on AROS).
    try_build("multi_thread + enable_all", || {
        Builder::new_multi_thread().worker_threads(2).enable_all().build()
    });
    println!("RUST-AROS: TOKIO PROBE DONE");
    0
}
