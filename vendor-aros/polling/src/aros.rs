//! AROS backend for `polling`.
//!
//! AROS (hosted) has no unified `poll()`/`select()` across its heterogeneous
//! descriptors — sockets live in `bsdsocket.library`, pipes/files in
//! `dos.library` — so this is a native software reactor built on `std` sync
//! primitives (which work on AROS). It correctly drives the async executor's
//! timers, task wake-ups, and `notify()`.
//!
//! Phase A (this): fd *readiness* is not yet reported — `wait` blocks until the
//! deadline or a `notify`. That is enough to compile the async stack and boot,
//! and to run any async work that isn't gated on fd I/O. Phase B wires real
//! socket readiness via bsdsocket `WaitSelect` (which waits on socket fd-sets
//! *and* an exec signal mask) and dos pipe readiness via message ports.

use std::collections::HashMap;
use std::io;
use std::os::fd::{AsRawFd, BorrowedFd};
use std::sync::{Condvar, Mutex};
use std::time::Instant;

use crate::{Event, PollMode};

type RawFd = std::os::raw::c_int;

/// Software reactor.
#[derive(Debug)]
pub struct Poller {
    /// Registered descriptors and their requested interest. Kept for the
    /// upcoming WaitSelect-based readiness (Phase B); unused for wake-ups today.
    registry: Mutex<HashMap<RawFd, Event>>,
    /// `notify()` sets this; `wait` consumes it and returns immediately.
    notified: Mutex<bool>,
    /// Signals `wait` on `notify()` (and on a timeout via `wait_timeout`).
    signal: Condvar,
}

impl Poller {
    pub fn new() -> io::Result<Poller> {
        Ok(Poller {
            registry: Mutex::new(HashMap::new()),
            notified: Mutex::new(false),
            signal: Condvar::new(),
        })
    }

    /// Level-triggered is the model we present (edge is unsupported).
    pub fn supports_level(&self) -> bool {
        true
    }

    pub fn supports_edge(&self) -> bool {
        false
    }

    pub fn add(&self, fd: RawFd, ev: Event, _mode: PollMode) -> io::Result<()> {
        self.registry.lock().unwrap().insert(fd, ev);
        Ok(())
    }

    pub fn modify(&self, fd: BorrowedFd<'_>, ev: Event, _mode: PollMode) -> io::Result<()> {
        self.registry.lock().unwrap().insert(fd.as_raw_fd(), ev);
        Ok(())
    }

    pub fn delete(&self, fd: BorrowedFd<'_>) -> io::Result<()> {
        self.registry.lock().unwrap().remove(&fd.as_raw_fd());
        Ok(())
    }

    /// Block until `notify` or `deadline`. Reports no fd events yet (Phase A).
    pub fn wait_deadline(
        &self,
        events: &mut Events,
        deadline: Option<Instant>,
    ) -> io::Result<()> {
        events.inner.clear();
        let mut notified = self.notified.lock().unwrap();
        loop {
            if *notified {
                *notified = false;
                return Ok(());
            }
            match deadline {
                None => {
                    notified = self.signal.wait(notified).unwrap();
                }
                Some(dl) => {
                    let now = Instant::now();
                    if now >= dl {
                        return Ok(());
                    }
                    let (guard, timeout) =
                        self.signal.wait_timeout(notified, dl - now).unwrap();
                    notified = guard;
                    if timeout.timed_out() {
                        return Ok(());
                    }
                }
            }
        }
    }

    pub fn notify(&self) -> io::Result<()> {
        *self.notified.lock().unwrap() = true;
        self.signal.notify_one();
        Ok(())
    }
}

/// Per-event extra flags (hangup / priority / error). Populated once Phase B
/// reports real readiness; `empty()` for now.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct EventExtra {
    hup: bool,
    pri: bool,
    err: bool,
}

impl EventExtra {
    pub const fn empty() -> Self {
        EventExtra {
            hup: false,
            pri: false,
            err: false,
        }
    }

    pub fn set_hup(&mut self, value: bool) {
        self.hup = value;
    }

    pub fn set_pri(&mut self, value: bool) {
        self.pri = value;
    }

    pub fn is_hup(&self) -> bool {
        self.hup
    }

    pub fn is_pri(&self) -> bool {
        self.pri
    }

    pub fn is_connect_failed(&self) -> Option<bool> {
        Some(self.err || self.hup)
    }

    pub fn is_err(&self) -> Option<bool> {
        Some(self.err)
    }
}

/// The list of events filled by `wait`. Empty until Phase B reports readiness.
#[derive(Debug)]
pub struct Events {
    inner: Vec<Event>,
}

impl Events {
    pub fn with_capacity(cap: usize) -> Events {
        Events {
            inner: Vec::with_capacity(cap),
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = Event> + '_ {
        self.inner.iter().copied()
    }

    pub fn clear(&mut self) {
        self.inner.clear();
    }

    pub fn capacity(&self) -> usize {
        self.inner.capacity()
    }
}
