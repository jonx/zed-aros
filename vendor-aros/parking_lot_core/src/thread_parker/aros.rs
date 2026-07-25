// Copyright 2016 Amanieu d'Antras
//
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// http://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

//! Thread parker for AROS, over `pthread.library` mutexes and condition
//! variables.
//!
//! AROS is not `cfg(unix)`, so without this the selection in `mod.rs` falls
//! through to `generic.rs`, whose `park()` is a bare `spin_loop()`. A parked
//! thread then burns its whole scheduling quantum, and since AROS tasks are
//! round-robin at equal priority, one spinning thread starves every other one.
//!
//! The pthread objects are owned by C (`aros_sync_glue.c`) and addressed
//! through zeroed, pinned byte buffers, matching how the AROS `std` sync pal
//! does it. `aros_cond_timedwait` takes a *relative* timeout and computes the
//! absolute deadline itself, so the timespec layout stays on the C side.

use core::{
    cell::{Cell, UnsafeCell},
    ffi::c_void,
};
use std::thread;
use std::time::Instant;

// pthread_mutex_t is 136 bytes and pthread_cond_t 152 on AROS aarch64. Both are
// over-sized here; `prepare_park` asserts the real sizes still fit.
const MUTEX_WORDS: usize = 20; // 160 bytes, 8-aligned
const COND_WORDS: usize = 22; // 176 bytes, 8-aligned

unsafe extern "C" {
    fn aros_mtx_size() -> usize;
    fn aros_mtx_init(m: *mut c_void) -> i32;
    fn aros_mtx_lock(m: *mut c_void) -> i32;
    fn aros_mtx_unlock(m: *mut c_void) -> i32;
    fn aros_mtx_destroy(m: *mut c_void) -> i32;

    fn aros_cond_size() -> usize;
    fn aros_cond_init(c: *mut c_void) -> i32;
    fn aros_cond_signal(c: *mut c_void) -> i32;
    fn aros_cond_wait(c: *mut c_void, m: *mut c_void) -> i32;
    fn aros_cond_timedwait(c: *mut c_void, m: *mut c_void, secs: u32, nsecs: u32) -> i32;
    fn aros_cond_destroy(c: *mut c_void) -> i32;
}

pub struct ThreadParker {
    should_park: Cell<bool>,
    mutex: UnsafeCell<[u64; MUTEX_WORDS]>,
    condvar: UnsafeCell<[u64; COND_WORDS]>,
    initialized: Cell<bool>,
}

impl ThreadParker {
    #[inline]
    fn mutex(&self) -> *mut c_void {
        self.mutex.get() as *mut c_void
    }

    #[inline]
    fn condvar(&self) -> *mut c_void {
        self.condvar.get() as *mut c_void
    }

    #[inline]
    unsafe fn init(&self) {
        assert!(
            unsafe { aros_mtx_size() } <= MUTEX_WORDS * 8
                && unsafe { aros_cond_size() } <= COND_WORDS * 8,
            "pthread mutex/cond larger than the parker buffers"
        );
        let r = unsafe { aros_mtx_init(self.mutex()) };
        debug_assert_eq!(r, 0);
        let r = unsafe { aros_cond_init(self.condvar()) };
        debug_assert_eq!(r, 0);
    }
}

impl super::ThreadParkerT for ThreadParker {
    type UnparkHandle = UnparkHandle;

    const IS_CHEAP_TO_CONSTRUCT: bool = false;

    #[inline]
    fn new() -> ThreadParker {
        ThreadParker {
            should_park: Cell::new(false),
            // A zeroed buffer is a valid PTHREAD_*_INITIALIZER; `init` sets both up.
            mutex: UnsafeCell::new([0; MUTEX_WORDS]),
            condvar: UnsafeCell::new([0; COND_WORDS]),
            initialized: Cell::new(false),
        }
    }

    #[inline]
    unsafe fn prepare_park(&self) {
        self.should_park.set(true);
        if !self.initialized.get() {
            unsafe { self.init() };
            self.initialized.set(true);
        }
    }

    #[inline]
    unsafe fn timed_out(&self) -> bool {
        // The mutex is needed here because another thread may concurrently be
        // in UnparkHandle::unpark, which runs without the queue lock.
        let r = unsafe { aros_mtx_lock(self.mutex()) };
        debug_assert_eq!(r, 0);
        let should_park = self.should_park.get();
        let r = unsafe { aros_mtx_unlock(self.mutex()) };
        debug_assert_eq!(r, 0);
        should_park
    }

    #[inline]
    unsafe fn park(&self) {
        let r = unsafe { aros_mtx_lock(self.mutex()) };
        debug_assert_eq!(r, 0);
        while self.should_park.get() {
            let r = unsafe { aros_cond_wait(self.condvar(), self.mutex()) };
            debug_assert_eq!(r, 0);
        }
        let r = unsafe { aros_mtx_unlock(self.mutex()) };
        debug_assert_eq!(r, 0);
    }

    #[inline]
    unsafe fn park_until(&self, timeout: Instant) -> bool {
        let r = unsafe { aros_mtx_lock(self.mutex()) };
        debug_assert_eq!(r, 0);
        while self.should_park.get() {
            let now = Instant::now();
            if timeout <= now {
                let r = unsafe { aros_mtx_unlock(self.mutex()) };
                debug_assert_eq!(r, 0);
                return false;
            }
            let rel = timeout - now;
            // The glue clamps the deadline to AROS's 32-bit time_t, so an
            // over-long wait becomes a very distant one rather than an
            // immediate ETIMEDOUT (which would busy-loop the caller).
            let secs = rel.as_secs().min(u32::MAX as u64) as u32;
            let r = unsafe {
                aros_cond_timedwait(self.condvar(), self.mutex(), secs, rel.subsec_nanos())
            };
            debug_assert!(r == 0 || r == ETIMEDOUT);
        }
        let r = unsafe { aros_mtx_unlock(self.mutex()) };
        debug_assert_eq!(r, 0);
        true
    }

    #[inline]
    unsafe fn unpark_lock(&self) -> UnparkHandle {
        let r = unsafe { aros_mtx_lock(self.mutex()) };
        debug_assert_eq!(r, 0);

        UnparkHandle {
            thread_parker: self,
        }
    }
}

// AROS posixc <errno.h>
const ETIMEDOUT: i32 = 60;

impl Drop for ThreadParker {
    #[inline]
    fn drop(&mut self) {
        if self.initialized.get() {
            unsafe {
                aros_mtx_destroy(self.mutex());
                aros_cond_destroy(self.condvar());
            }
        }
    }
}

pub struct UnparkHandle {
    thread_parker: *const ThreadParker,
}

impl super::UnparkHandleT for UnparkHandle {
    #[inline]
    unsafe fn unpark(self) {
        unsafe {
            (*self.thread_parker).should_park.set(false);

            // Signal while still holding the lock: the target thread could exit
            // once the mutex is released, which would make the condvar access
            // invalid memory.
            let r = aros_cond_signal((*self.thread_parker).condvar());
            debug_assert_eq!(r, 0);
            let r = aros_mtx_unlock((*self.thread_parker).mutex());
            debug_assert_eq!(r, 0);
        }
    }
}

#[inline]
pub fn thread_yield() {
    thread::yield_now();
}
