//! Task dispatcher for the AROS backend.
//!
//! std threads are proven on hosted AROS, so this is a plain std-threaded
//! dispatcher: a background worker pool draining a priority queue, a main-thread
//! FIFO drained by the platform run loop, and a single long-lived timer thread
//! servicing all `dispatch_after` deadlines from a min-heap.

use std::cmp::Ordering;
use std::ffi::c_int;
use std::collections::{BinaryHeap, VecDeque};
use std::sync::Arc;
use std::thread::{self, ThreadId};
use std::time::{Duration, Instant};

use gpui::{PlatformDispatcher, Priority, RunnableVariant};
use parking_lot::Mutex;
// std's Mutex/Condvar (NOT parking_lot's) for anything that BLOCKS: on this
// target parking_lot_core falls back to its generic ThreadParker, which
// busy-spins instead of parking -- fatal on a single-CPU cooperative guest
// (measured: 99% CPU at idle). std's condvar routes through aros_cond_wait ->
// pthread_cond_wait -> a real exec Wait().
use std::sync::{Condvar as StdCondvar, Mutex as StdMutex};

use crate::glue;

/// A blocking MPMC queue.
///
/// Deliberately NOT crossbeam_channel: crossbeam is lock-free and spins
/// (`Backoff::snooze()` -> `yield_now()`) before parking, which is designed for
/// real multi-core parallelism. Hosted AROS multiplexes EVERY task onto one
/// host CPU with cooperative, signal-based switching -- so a spinning receiver
/// cannot make progress, it can only burn the CPU that the task it is waiting
/// for needs to run. Task dumps during a freeze caught exactly that: a worker
/// pinned at 100% in `crossbeam_channel::flavors::list::Channel::recv` while
/// the UI sat in READY. A mutex+condvar blocks immediately and hands the CPU
/// back.
struct BlockingQueue<T> {
    items: StdMutex<Option<VecDeque<T>>>, // None => closed
    ready: StdCondvar,
}

impl<T> BlockingQueue<T> {
    fn new() -> Self {
        Self {
            items: StdMutex::new(Some(VecDeque::new())),
            ready: StdCondvar::new(),
        }
    }

    /// Returns false if the queue is closed.
    fn push(&self, item: T) -> bool {
        let mut guard = self.items.lock().unwrap_or_else(|e| e.into_inner());
        match guard.as_mut() {
            Some(q) => {
                q.push_back(item);
                drop(guard);
                self.ready.notify_one();
                true
            }
            None => false,
        }
    }

    /// Block until an item arrives; None once closed and drained.
    fn pop(&self) -> Option<T> {
        let mut guard = self.items.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            match guard.as_mut() {
                Some(q) => match q.pop_front() {
                    Some(item) => return Some(item),
                    None => {
                        guard = self
                            .ready
                            .wait(guard)
                            .unwrap_or_else(|e| e.into_inner());
                    }
                },
                None => return None,
            }
        }
    }

    /// Block until an item arrives or `timeout` elapses.
    fn pop_timeout(&self, timeout: Duration) -> Option<T> {
        let mut guard = self.items.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(item) = guard.as_mut()?.pop_front() {
            return Some(item);
        }
        guard = self
            .ready
            .wait_timeout(guard, timeout)
            .unwrap_or_else(|e| e.into_inner())
            .0;
        guard.as_mut()?.pop_front()
    }

    fn close(&self) {
        *self.items.lock().unwrap_or_else(|e| e.into_inner()) = None;
        self.ready.notify_all();
    }
}

const MIN_THREADS: usize = 2;

/// Exec priority for background workers and the timer thread: just below the
/// UI task (which runs at 0), so the UI always preempts background work on the
/// single guest CPU. Not lower than -1: these still need to outrank idle-ish
/// system tasks and make timely progress.
const WORKER_PRI: c_int = -1;

/// One pending `dispatch_after` runnable, ordered by deadline for the timer
/// thread's min-heap. `seq` breaks ties (and keeps the ordering total without
/// having to order `RunnableVariant`); `BinaryHeap` is a max-heap, so both
/// comparisons are reversed to pop the *earliest* deadline first.
struct TimerEntry {
    deadline: Instant,
    seq: u64,
    runnable: RunnableVariant,
}

impl PartialEq for TimerEntry {
    fn eq(&self, other: &Self) -> bool {
        self.deadline == other.deadline && self.seq == other.seq
    }
}
impl Eq for TimerEntry {}
impl PartialOrd for TimerEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for TimerEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        self.deadline
            .cmp(&other.deadline)
            .then(self.seq.cmp(&other.seq))
            .reverse()
    }
}

pub(crate) struct ArosDispatcher {
    main_thread_id: ThreadId,
    main_queue: Arc<Mutex<VecDeque<RunnableVariant>>>,
    background_queue: Arc<BlockingQueue<RunnableVariant>>,
    timer_queue: Arc<BlockingQueue<(Instant, RunnableVariant)>>,
    _background_threads: Vec<thread::JoinHandle<()>>,
    _timer_thread: thread::JoinHandle<()>,
}

impl Drop for ArosDispatcher {
    fn drop(&mut self) {
        self.background_queue.close();
        self.timer_queue.close();
    }
}

impl ArosDispatcher {
    /// Must be constructed on the main thread (records its `ThreadId`).
    pub(crate) fn new() -> Self {
        // gpui's PriorityQueue re-export is not available for this target, so a
        // plain blocking MPMC queue backs the worker pool. Priority is not
        // honored; the run loop's frame cadence keeps latency bounded
        // (correctness first).
        let background_queue: Arc<BlockingQueue<RunnableVariant>> =
            Arc::new(BlockingQueue::new());
        // Every AROS task -- including these workers -- is multiplexed onto a
        // SINGLE host thread by the hosted kernel, so `available_parallelism()`
        // (the host's core count, ~10-14 on an Apple Silicon Mac) buys exactly
        // zero parallelism here. It only multiplies context switches (each one
        // a raise()/sigsuspend() signal round-trip) and contention on the
        // shared, not-always-thread-safe C runtime (posixc's fd table, the
        // timer device). Cap it: two workers keep I/O overlapping with the UI
        // without spawning a herd that just fights over one CPU.
        let thread_count = if cfg!(target_os = "aros") {
            MIN_THREADS
        } else {
            thread::available_parallelism().map_or(MIN_THREADS, |n| n.get().max(MIN_THREADS))
        };

        let background_threads = (0..thread_count)
            .map(|i| {
                let queue = background_queue.clone();
                thread::Builder::new()
                    .name(format!("gpui-aros-worker-{i}"))
                    .spawn(move || {
                        // Below the UI task (see gpa_lower_task_pri): with one
                        // guest CPU and exec's strict-priority scheduling, an
                        // equal-priority worker starves the UI outright.
                        // SAFETY: FFI; acts on the calling task only.
                        unsafe { glue::gpa_lower_task_pri(WORKER_PRI) };
                        while let Some(runnable) = queue.pop() {
                            runnable.run();
                        }
                    })
                    .expect("failed to spawn gpui_aros background worker")
            })
            .collect::<Vec<_>>();

        let main_queue = Arc::new(Mutex::new(VecDeque::new()));

        // One long-lived timer thread services every `dispatch_after` from a
        // deadline-ordered heap, instead of spawning a fresh sleeper task per
        // call — gpui fires timers constantly (animation, debounce, `Timer`),
        // and on hosted AROS each thread is an exec Task from a finite pool, so
        // thread-per-timer churned tasks and risked a spawn abort on the hot
        // path (fatal in AROS's single address space).
        let timer_queue: Arc<BlockingQueue<(Instant, RunnableVariant)>> =
            Arc::new(BlockingQueue::new());
        let timer_thread = {
            let main_queue = main_queue.clone();
            let timer_queue = timer_queue.clone();
            thread::Builder::new()
                .name("gpui-aros-timer".to_owned())
                .spawn(move || {
                    // Same rationale as the workers: never outrank the UI.
                    // SAFETY: FFI; acts on the calling task only.
                    unsafe { glue::gpa_lower_task_pri(WORKER_PRI) };
                    timer_loop(timer_queue, main_queue)
                })
                .expect("failed to spawn gpui_aros timer thread")
        };

        Self {
            main_thread_id: thread::current().id(),
            main_queue,
            background_queue,
            timer_queue,
            _background_threads: background_threads,
            _timer_thread: timer_thread,
        }
    }

    /// Shared handle to the main-thread queue, drained by the run loop.
    pub(crate) fn main_queue(&self) -> Arc<Mutex<VecDeque<RunnableVariant>>> {
        self.main_queue.clone()
    }
}

impl PlatformDispatcher for ArosDispatcher {
    fn is_main_thread(&self) -> bool {
        thread::current().id() == self.main_thread_id
    }

    fn dispatch(&self, runnable: RunnableVariant, _priority: Priority) {
        if !self.background_queue.push(runnable) {
            log::error!("gpui_aros: background dispatch failed (workers gone)");
        }
    }

    fn dispatch_on_main_thread(&self, runnable: RunnableVariant, _priority: Priority) {
        self.main_queue.lock().push_back(runnable);
        // Nudge the run loop out of its park so the work starts within the
        // poll granularity (~2 ms) instead of the frame budget. Safe from
        // any thread (hosted-AROS threads are exec tasks); no-op pre-init.
        unsafe { glue::gpa_wake_main() };
    }

    fn dispatch_after(&self, duration: Duration, runnable: RunnableVariant) {
        let deadline = Instant::now() + duration;
        // Hand the deadline to the shared timer thread. Failure only happens if
        // that thread is gone (shutdown) — drop the runnable rather than panic,
        // matching `dispatch`.
        if !self.timer_queue.push((deadline, runnable)) {
            log::error!("gpui_aros: timer dispatch failed (timer thread gone)");
        }
    }

    fn spawn_realtime(&self, f: Box<dyn FnOnce() + Send>) {
        thread::spawn(f);
    }
}

/// The single timer thread: keep a min-heap of pending deadlines, park until
/// the nearest one (or until a new deadline arrives), then push every matured
/// runnable onto the main queue and wake the run loop. Exits when the sender is
/// dropped (dispatcher teardown).
fn timer_loop(
    queue: Arc<BlockingQueue<(Instant, RunnableVariant)>>,
    main_queue: Arc<Mutex<VecDeque<RunnableVariant>>>,
) {
    let mut heap: BinaryHeap<TimerEntry> = BinaryHeap::new();
    let mut seq: u64 = 0;
    loop {
        // Fire everything already due.
        let now = Instant::now();
        let mut woke = false;
        while heap.peek().is_some_and(|e| e.deadline <= now) {
            let entry = heap.pop().expect("peeked");
            main_queue.lock().push_back(entry.runnable);
            woke = true;
        }
        if woke {
            // SAFETY: see dispatch_on_main_thread; safe from any thread.
            unsafe { glue::gpa_wake_main() };
        }

        // Park until the next deadline, or indefinitely if none pending.
        // A zero/near-zero timeout would busy-loop, so only wait when the
        // deadline is still ahead; otherwise loop straight back and fire it.
        let received = match heap.peek() {
            Some(next) => {
                let timeout = next.deadline.saturating_duration_since(Instant::now());
                if timeout.is_zero() {
                    continue;
                }
                match queue.pop_timeout(timeout) {
                    Some(item) => Some(item),
                    // Timed out (or spurious): loop back and fire matured deadlines.
                    None => continue,
                }
            }
            // Nothing pending: block. `None` means the queue closed (shutdown).
            None => match queue.pop() {
                Some(item) => Some(item),
                None => return,
            },
        };
        if let Some((deadline, runnable)) = received {
            heap.push(TimerEntry {
                deadline,
                seq,
                runnable,
            });
            seq = seq.wrapping_add(1);
        }
    }
}
