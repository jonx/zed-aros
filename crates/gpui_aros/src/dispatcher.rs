//! Task dispatcher for the AROS backend.
//!
//! std threads are proven on hosted AROS, so this is a plain std-threaded
//! dispatcher: a background worker pool draining a priority queue, a main-thread
//! FIFO drained by the platform run loop, and one sleeper thread per timer.

use std::collections::VecDeque;
use std::sync::Arc;
use std::thread::{self, ThreadId};
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender, unbounded};
use gpui::{PlatformDispatcher, Priority, RunnableVariant};
use parking_lot::Mutex;

use crate::glue;

const MIN_THREADS: usize = 2;

pub(crate) struct ArosDispatcher {
    main_thread_id: ThreadId,
    main_queue: Arc<Mutex<VecDeque<RunnableVariant>>>,
    background_sender: Sender<RunnableVariant>,
    _background_threads: Vec<thread::JoinHandle<()>>,
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

        Self {
            main_thread_id: thread::current().id(),
            main_queue: Arc::new(Mutex::new(VecDeque::new())),
            background_sender,
            _background_threads: background_threads,
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
        let main_queue = self.main_queue.clone();
        thread::Builder::new()
            .name("gpui-aros-timer".to_owned())
            .spawn(move || {
                thread::sleep(duration);
                main_queue.lock().push_back(runnable);
                // SAFETY: see dispatch_on_main_thread.
                unsafe { glue::gpa_wake_main() };
            })
            .expect("failed to spawn gpui_aros timer thread");
    }

    fn spawn_realtime(&self, f: Box<dyn FnOnce() + Send>) {
        thread::spawn(f);
    }
}
