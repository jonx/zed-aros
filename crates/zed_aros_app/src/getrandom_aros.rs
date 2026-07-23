//! getrandom custom backend for AROS: forward to posixc `arc4random_buf`
//! (a real CSPRNG since the entropy.resource merge; infallible by contract).
//! Selected by `--cfg getrandom_backend="custom"` in .cargo/config.toml.
#![cfg(target_os = "aros")]

use core::ffi::c_void;

extern "C" {
    fn arc4random_buf(buf: *mut c_void, nbytes: usize);
}

// getrandom 0.3 / 0.4 custom backend (cfg getrandom_backend="custom").
#[no_mangle]
unsafe extern "Rust" fn __getrandom_v03_custom(
    dest: *mut u8,
    len: usize,
) -> Result<(), getrandom::Error> {
    arc4random_buf(dest.cast(), len);
    Ok(())
}

// getrandom 0.2 custom backend (registered via its macro -> __getrandom_custom).
getrandom02::register_custom_getrandom!(aros_getrandom_02);
fn aros_getrandom_02(buf: &mut [u8]) -> Result<(), getrandom02::Error> {
    unsafe { arc4random_buf(buf.as_mut_ptr().cast(), buf.len()) };
    Ok(())
}
