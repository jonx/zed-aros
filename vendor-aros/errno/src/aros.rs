//! AROS errno accessor. Self-contained: the `libc` crate has no AROS target,
//! so this declares the two posixc symbols it needs directly (the same pattern
//! the AROS Rust `std` pal uses). errno lives at `*__stdc_geterrnoptr()`;
//! messages come from posixc `strerror_r`.

use crate::Errno;
use core::ffi::{c_char, c_int};
use core::str;

extern "C" {
    fn __stdc_geterrnoptr() -> *mut c_int;
    fn strerror_r(errnum: c_int, buf: *mut c_char, buflen: usize) -> c_int;
}

pub fn with_description<F, T>(err: Errno, callback: F) -> T
where
    F: FnOnce(Result<&str, Errno>) -> T,
{
    let mut buf = [0u8; 1024];
    let rc = unsafe { strerror_r(err.0, buf.as_mut_ptr() as *mut c_char, buf.len()) };
    if rc != 0 {
        return callback(Err(err));
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    callback(str::from_utf8(&buf[..end]).map_err(|_| err))
}

pub const STRERROR_NAME: &str = "strerror_r";

pub fn errno() -> Errno {
    unsafe { Errno(*__stdc_geterrnoptr()) }
}

pub fn set_errno(Errno(errno): Errno) {
    unsafe {
        *__stdc_geterrnoptr() = errno;
    }
}
