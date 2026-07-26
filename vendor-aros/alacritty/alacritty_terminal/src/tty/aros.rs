//! Terminal backend for AROS, over pipes rather than a pseudo-terminal.
//!
//! AROS has no PTY device, so there is no controlling terminal to give a child:
//! no line discipline, no job control, and no size to notify it about. What it
//! does have (since the streaming `std::process` work) is a child whose stdio is
//! connected to live `PIPE:` endpoints, which is enough to run a shell, type at
//! it, and watch its output appear.
//!
//! What that means in practice:
//!   - commands and their output work;
//!   - there is no prompt: the shell does not know it is interactive;
//!   - full-screen programs do not work, because nothing reflows them and they
//!     see no terminal size;
//!   - `on_resize` is inert for the same reason.
//!
//! A real PTY would replace this wholesale (item 3 of the port's
//! os-requirements). Until then this is a working terminal rather than none.
//!
//! Readiness: an AROS pipe endpoint is a dos filehandle, not an fd, so it cannot
//! be handed to the poller. A reader thread does blocking reads into a shared
//! buffer instead, and the `Read` impl drains that buffer, reporting
//! `WouldBlock` when it is empty -- which is what the event loop expects from a
//! non-blocking source. Nothing can be *registered* with the poller, but it can
//! still be woken, so `register` keeps it and the reader thread notifies it once
//! there is something to show. The event loop also ticks on a timer
//! (`AROS_TICK`), which is what notices the child exiting.
//!
//! Line discipline: there is no tty to do it, so `PtyWriter` does the two parts
//! that are load-bearing -- echoing what was typed, and turning the terminal's
//! `\r` into the `\n` a shell reads as end of line.

use std::borrow::Cow;
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
    /// Set once the event loop registers. Nothing here can be *added* to a
    /// poller, but it can still be woken, which is how output reaches the
    /// screen when the terminal is idle rather than waiting for the next
    /// keystroke to shake it loose.
    waker: Option<Arc<Poller>>,
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

/// The child's stdin, plus the two jobs a tty's line discipline would be doing.
///
/// A terminal sends `\r` for Return and expects the tty to echo what was typed;
/// there is no tty here, so the shell would see a line that never ends and the
/// user would see nothing at all. So Return is translated to `\n` on the way to
/// the child, and everything written is echoed back into the same buffer the
/// child's output arrives in.
pub struct PtyWriter {
    stdin: ChildStdin,
    inbox: Arc<Mutex<Inbox>>,
}

impl Write for PtyWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let line_ends = buf.iter().any(|&b| b == b'\r');
        let translated: Cow<'_, [u8]> = if line_ends {
            Cow::Owned(buf.iter().map(|&b| if b == b'\r' { b'\n' } else { b }).collect())
        } else {
            Cow::Borrowed(buf)
        };
        self.stdin.write_all(&translated)?;
        // A shell only acts on a whole line, so push it through rather than
        // leaving it in a buffer the child cannot see.
        if line_ends {
            self.stdin.flush()?;
        }

        let mut inbox = self.inbox.lock().unwrap_or_else(|e| e.into_inner());
        for &b in buf {
            // The grid needs both halves to put the cursor at the next line.
            if b == b'\r' {
                inbox.buf.extend(b"\r\n");
            } else {
                inbox.buf.push_back(b);
            }
        }
        Ok(buf.len())
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
    // AROS has no shell to run as a program: the shell is a resident segment,
    // and there is no `C:Shell` to start. What AROS understands is a command
    // line with no command in it, which starts a shell that reads the input it
    // was given -- an interactive shell on our pipes, which is what a terminal
    // is. So the default here is no program at all.
    //
    // Zed has no way to know that and asks for `/bin/sh` (there being no
    // `$SHELL` to read), so a unix shell path means "just give me a shell".
    let (program, args) = match &config.shell {
        Some(shell) if !shell.program.starts_with('/') => {
            (shell.program.clone(), shell.args.clone())
        }
        _ => (String::new(), Vec::new()),
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
        reader: PtyReader { inbox: inbox.clone() },
        writer: PtyWriter { stdin, inbox },
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
        // The rest of the line discipline: a terminal takes `\n` as "down one
        // line" and nothing more, so without this every line starts under the
        // end of the one before it. AROS writes bare newlines, so put the
        // carriage return back -- unless the child sent one already.
        let mut after_cr = false;
        loop {
            match src.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => {
                    let mut guard = inbox.lock().unwrap_or_else(|e| e.into_inner());
                    for &b in &chunk[..n] {
                        if b == b'\n' && !after_cr {
                            guard.buf.push_back(b'\r');
                        }
                        guard.buf.push_back(b);
                        after_cr = b == b'\r';
                    }
                    if let Some(waker) = &guard.waker {
                        let _ = waker.notify();
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
        if marks_eof {
            let mut guard = inbox.lock().unwrap_or_else(|e| e.into_inner());
            guard.eof = true;
            running.store(false, Ordering::SeqCst);
            if let Some(waker) = &guard.waker {
                let _ = waker.notify();
            }
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

    unsafe fn register(&mut self, poller: &Arc<Poller>, _: Event, _: PollMode) -> io::Result<()> {
        // A dos filehandle cannot be registered with the poller. Keep the
        // poller anyway, so a pump thread can wake the event loop when the
        // child says something.
        self.reader.inbox.lock().unwrap_or_else(|e| e.into_inner()).waker =
            Some(poller.clone());
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
