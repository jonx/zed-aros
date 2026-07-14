//! Task dispatcher for the AROS backend.
//!
//! std threads are proven on hosted AROS, so this is a plain std-threaded
//! dispatcher: a background worker pool draining a priority queue, a main-thread
//! FIFO drained by the platform run loop, and a single long-lived timer thread
//! servicing all `dispatch_after` deadlines from a min-heap.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, VecDeque};
use std::sync::Arc;
use std::thread::{self, ThreadId};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, unbounded};
use gpui::{PlatformDispatcher, Priority, RunnableVariant};
use parking_lot::Mutex;

use crate::glue;

const MIN_THREADS: usize = 2;

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
    background_sender: Sender<RunnableVariant>,
    timer_sender: Sender<(Instant, RunnableVariant)>,
    _background_threads: Vec<thread::JoinHandle<()>>,
    _timer_thread: thread::JoinHandle<()>,
}

impl ArosDispatcher {
    /// Must be constructed on the main thread (records its `ThreadId`).
    pub(crate) fn new() -> Self {
        // gpui's PriorityQueue re-export is not available for this target, so a
        // plain MPMC channel backs the worker pool. Priority is not honored; the
        // run loop's frame cadence keeps latency bounded (correctness first).
        let (background_sender, background_receiver): (
            Sender<RunnableVariant>,
            Receiver<RunnableVariant>,
        ) = unbounded();
        let thread_count =
            thread::available_parallelism().map_or(MIN_THREADS, |n| n.get().max(MIN_THREADS));

        let background_threads = (0..thread_count)
            .map(|i| {
                let receiver = background_receiver.clone();
                thread::Builder::new()
                    .name(format!("gpui-aros-worker-{i}"))
                    .spawn(move || {
                        for runnable in receiver.iter() {
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
        let (timer_sender, timer_receiver) = unbounded::<(Instant, RunnableVariant)>();
        let timer_thread = {
            let main_queue = main_queue.clone();
            thread::Builder::new()
                .name("gpui-aros-timer".to_owned())
                .spawn(move || timer_loop(timer_receiver, main_queue))
                .expect("failed to spawn gpui_aros timer thread")
        };

        Self {
            main_thread_id: thread::current().id(),
            main_queue,
            background_sender,
            timer_sender,
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
        if self.background_sender.send(runnable).is_err() {
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
        if self.timer_sender.send((deadline, runnable)).is_err() {
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
    receiver: Receiver<(Instant, RunnableVariant)>,
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
        let received = match heap.peek() {
            Some(next) => {
                let timeout = next.deadline.saturating_duration_since(Instant::now());
                receiver.recv_timeout(timeout)
            }
            None => receiver.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        match received {
            Ok((deadline, runnable)) => {
                heap.push(TimerEntry {
                    deadline,
                    seq,
                    runnable,
                });
                seq = seq.wrapping_add(1);
            }
            // Timed out: loop back and fire matured deadlines.
            Err(RecvTimeoutError::Timeout) => {}
            // Sender dropped (shutdown): stop; any pending timers are abandoned.
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}
