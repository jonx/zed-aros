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
use std::time::{Duration, Instant};

use crate::{Event, PollMode};

type RawFd = std::os::raw::c_int;

// The reactor caps each WaitSelect to this so notify() latency is bounded (a
// notify is seen at the next loop turn). Socket readiness itself is prompt --
// WaitSelect returns as soon as a socket is ready, within the window.
const POLL_INTERVAL_US: i64 = 50_000;

unsafe extern "C" {
    // WaitSelect-based socket readiness (aros_net_glue.c). Returns >=0 ready
    // count / -1 error; fills `got` (bit0=readable, bit1=writable).
    fn aros_np_waitselect(
        n: std::os::raw::c_int,
        fds: *const RawFd,
        want: *const u8,
        got: *mut u8,
        timeout_us: i64,
    ) -> std::os::raw::c_int;
}

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

    /// Wait for socket readiness, `notify`, or `deadline` (Phase B). Registered
    /// sockets are polled via bsdsocket `WaitSelect`; with none registered this
    /// falls back to the condvar wait (timers/notify only).
    pub fn wait_deadline(
        &self,
        events: &mut Events,
        deadline: Option<Instant>,
    ) -> io::Result<()> {
        events.inner.clear();
        loop {
            // A pending notify wins immediately.
            {
                let mut notified = self.notified.lock().unwrap();
                if *notified {
                    *notified = false;
                    return Ok(());
                }
            }
            let now = Instant::now();
            if let Some(dl) = deadline {
                if now >= dl {
                    return Ok(());
                }
            }

            // This turn's timeout, capped so a notify is seen promptly.
            let remaining_us = match deadline {
                None => POLL_INTERVAL_US,
                Some(dl) => (dl.saturating_duration_since(now).as_micros() as i64)
                    .min(POLL_INTERVAL_US),
            };

            let snapshot: Vec<(RawFd, Event)> = {
                let reg = self.registry.lock().unwrap();
                reg.iter().map(|(&fd, &ev)| (fd, ev)).collect()
            };

            if snapshot.is_empty() {
                // No sockets to poll: condvar wait (wakes at once on notify).
                let notified = self.notified.lock().unwrap();
                if *notified {
                    continue;
                }
                let dur = Duration::from_micros(remaining_us.max(0) as u64);
                let _ = self.signal.wait_timeout(notified, dur).unwrap();
                continue;
            }

            let fds: Vec<RawFd> = snapshot.iter().map(|(fd, _)| *fd).collect();
            let want: Vec<u8> = snapshot
                .iter()
                .map(|(_, ev)| (ev.readable as u8) | ((ev.writable as u8) << 1))
                .collect();
            let mut got = vec![0u8; snapshot.len()];
            let r = unsafe {
                aros_np_waitselect(
                    fds.len() as std::os::raw::c_int,
                    fds.as_ptr(),
                    want.as_ptr(),
                    got.as_mut_ptr(),
                    remaining_us,
                )
            };
            if r < 0 {
                // WaitSelect error: brief back-off, then retry.
                std::thread::sleep(Duration::from_millis(5));
                continue;
            }

            for (i, (_, ev)) in snapshot.iter().enumerate() {
                let g = got[i];
                if g != 0 {
                    events.inner.push(Event {
                        key: ev.key,
                        readable: g & 1 != 0,
                        writable: g & 2 != 0,
                        extra: EventExtra::empty(),
                    });
                }
            }
            if !events.inner.is_empty() {
                return Ok(());
            }
            // Timed out with nothing ready: loop (re-check notify + deadline).
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
