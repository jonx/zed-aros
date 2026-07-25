//! Terminal backend for AROS, over pipes rather than a pseudo-terminal.
//!
//! AROS has no PTY device, so there is no controlling terminal to give a child:
//! no line discipline, no job control, and no size to notify it about. What it
//! does have (since the streaming `std::process` work) is a child whose stdio is
//! connected to live `PIPE:` endpoints, which is enough to run a shell, type at
//! it, and watch its output appear.
//!
//! What that means in practice:
//!   - a shell prompt, commands, and their output all work;
//!   - full-screen programs do not, because nothing reflows them and they see
//!     no terminal size;
//!   - `on_resize` is inert for the same reason.
//!
//! A real PTY would replace this wholesale (item 3 of the port's
//! os-requirements). Until then this is a working terminal rather than none.
//!
//! Readiness: an AROS pipe endpoint is a dos filehandle, not an fd, so it cannot
//! be handed to the poller. A reader thread does blocking reads into a shared
//! buffer instead, and the `Read` impl drains that buffer, reporting
//! `WouldBlock` when it is empty -- which is what the event loop expects from a
//! non-blocking source.

use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::os::fd::OwnedFd;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use polling::{Event, PollMode, Poller};

use crate::event::{OnResize, WindowSize};
use crate::tty::{ChildEvent, EventedPty, EventedReadWrite, Options};

pub(crate) const PTY_READ_WRITE_TOKEN: usize = 0;
pub(crate) const PTY_CHILD_EVENT_TOKEN: usize = 1;

// AROS has no POSIX signal masks; a stub so the Options field type resolves.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SignalMask;

impl SignalMask {
    pub fn current() -> std::io::Result<Self> {
        Ok(SignalMask)
    }
}

/// Bytes a pump thread has pulled off the child but the event loop has not
/// consumed yet.
#[derive(Default)]
struct Inbox {
    buf: VecDeque<u8>,
    eof: bool,
}

pub struct PtyReader {
    inbox: Arc<Mutex<Inbox>>,
}

impl Read for PtyReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let mut inbox = self.inbox.lock().unwrap_or_else(|e| e.into_inner());
        if inbox.buf.is_empty() {
            // A 0-length read means EOF; an empty-but-open pipe must not.
            return if inbox.eof {
                Ok(0)
            } else {
                Err(io::Error::new(io::ErrorKind::WouldBlock, "no terminal output pending"))
            };
        }
        let n = out.len().min(inbox.buf.len());
        for slot in out.iter_mut().take(n) {
            *slot = inbox.buf.pop_front().expect("checked len");
        }
        Ok(n)
    }
}

pub struct PtyWriter {
    stdin: ChildStdin,
}

impl Write for PtyWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.stdin.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.stdin.flush()
    }
}

pub struct Pty {
    child: Child,
    reader: PtyReader,
    writer: PtyWriter,
    /// So the exit is reported exactly once.
    reported_exit: bool,
    running: Arc<AtomicBool>,
}

fn unsupported(what: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::Unsupported, what)
}

pub fn new(config: &Options, _window_size: WindowSize, _window_id: u64) -> io::Result<Pty> {
    // AROS has no $SHELL; the boot shell is the shell.
    let (program, args) = match &config.shell {
        Some(shell) => (shell.program.clone(), shell.args.clone()),
        None => ("C:Shell".to_owned(), Vec::new()),
    };

    let mut cmd = Command::new(&program);
    cmd.args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        // Fold stderr in too: a real tty shows both on one screen.
        .stderr(Stdio::piped());
    if let Some(dir) = &config.working_directory {
        cmd.current_dir(dir);
    }
    for (k, v) in &config.env {
        cmd.env(k, v);
    }

    let mut child = cmd.spawn()?;
    let stdin = child.stdin.take().ok_or_else(|| unsupported("terminal has no stdin"))?;
    let stdout = child.stdout.take().ok_or_else(|| unsupported("terminal has no stdout"))?;
    let stderr = child.stderr.take();

    let inbox: Arc<Mutex<Inbox>> = Arc::default();
    let running = Arc::new(AtomicBool::new(true));

    spawn_pump(stdout, inbox.clone(), running.clone(), true);
    if let Some(stderr) = stderr {
        spawn_pump(stderr, inbox.clone(), running.clone(), false);
    }

    Ok(Pty {
        child,
        reader: PtyReader { inbox },
        writer: PtyWriter { stdin },
        reported_exit: false,
        running,
    })
}

/// Blocking-read one of the child's output streams into the shared buffer.
///
/// Only the stdout pump marks EOF: stderr often closes first and must not end
/// the terminal early.
fn spawn_pump<R: Read + Send + 'static>(
    mut src: R,
    inbox: Arc<Mutex<Inbox>>,
    running: Arc<AtomicBool>,
    marks_eof: bool,
) {
    std::thread::spawn(move || {
        let mut chunk = [0u8; 4096];
        loop {
            match src.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => {
                    let mut guard = inbox.lock().unwrap_or_else(|e| e.into_inner());
                    guard.buf.extend(&chunk[..n]);
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
        if marks_eof {
            let mut guard = inbox.lock().unwrap_or_else(|e| e.into_inner());
            guard.eof = true;
            running.store(false, Ordering::SeqCst);
        }
    });
}

pub fn from_fd(
    _config: &Options,
    _window_id: u64,
    _master: OwnedFd,
    _slave: OwnedFd,
) -> io::Result<Pty> {
    // There is no descriptor to adopt: an AROS pipe endpoint is a dos filehandle.
    Err(unsupported("adopting a terminal from a descriptor is not possible on AROS"))
}

impl EventedReadWrite for Pty {
    type Reader = PtyReader;
    type Writer = PtyWriter;

    unsafe fn register(&mut self, _: &Arc<Poller>, _: Event, _: PollMode) -> io::Result<()> {
        // A dos filehandle cannot be registered with the poller; the pump
        // threads are what make output appear.
        Ok(())
    }

    fn reregister(&mut self, _: &Arc<Poller>, _: Event, _: PollMode) -> io::Result<()> {
        Ok(())
    }

    fn deregister(&mut self, _: &Arc<Poller>) -> io::Result<()> {
        Ok(())
    }

    fn reader(&mut self) -> &mut PtyReader {
        &mut self.reader
    }

    fn writer(&mut self) -> &mut PtyWriter {
        &mut self.writer
    }
}

impl EventedPty for Pty {
    fn next_child_event(&mut self) -> Option<ChildEvent> {
        if self.reported_exit {
            return None;
        }
        match self.child.try_wait() {
            Ok(Some(status)) => {
                self.reported_exit = true;
                Some(ChildEvent::Exited(Some(status)))
            }
            Ok(None) => None,
            Err(_) => {
                self.reported_exit = true;
                Some(ChildEvent::Exited(None))
            }
        }
    }
}

impl OnResize for Pty {
    fn on_resize(&mut self, _window_size: WindowSize) {
        // Nothing to tell: with no terminal device the child has no size, and no
        // way to be notified of one.
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        // Dropping the writer closes the child's stdin, which is the only way to
        // ask an AROS process to wind up: there is no kill.
        self.running.store(false, Ordering::SeqCst);
    }
}
