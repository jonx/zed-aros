//! AROS has no pseudo-terminal; the terminal is stubbed on this hosted port.
//! `Pty::new` always fails and the event impls are inert, so terminal-backed
//! features are gracefully unavailable rather than blocking the build.

use std::io::{self, Empty, Sink};
use std::os::fd::OwnedFd;
use std::sync::Arc;

use polling::{Event, PollMode, Poller};

use crate::event::{OnResize, WindowSize};
use crate::tty::{ChildEvent, EventedPty, EventedReadWrite, Options};

pub struct Pty {
    reader: Empty,
    writer: Sink,
}

fn unsupported() -> io::Error {
    io::Error::new(io::ErrorKind::Unsupported, "pseudo-terminals are not available on AROS")
}

pub fn new(_config: &Options, _window_size: WindowSize, _window_id: u64) -> io::Result<Pty> {
    Err(unsupported())
}

pub fn from_fd(
    _config: &Options,
    _window_id: u64,
    _master: OwnedFd,
    _slave: OwnedFd,
) -> io::Result<Pty> {
    Err(unsupported())
}

impl EventedReadWrite for Pty {
    type Reader = Empty;
    type Writer = Sink;

    unsafe fn register(&mut self, _: &Arc<Poller>, _: Event, _: PollMode) -> io::Result<()> {
        Ok(())
    }

    fn reregister(&mut self, _: &Arc<Poller>, _: Event, _: PollMode) -> io::Result<()> {
        Ok(())
    }

    fn deregister(&mut self, _: &Arc<Poller>) -> io::Result<()> {
        Ok(())
    }

    fn reader(&mut self) -> &mut Empty {
        &mut self.reader
    }

    fn writer(&mut self) -> &mut Sink {
        &mut self.writer
    }
}

impl EventedPty for Pty {
    fn next_child_event(&mut self) -> Option<ChildEvent> {
        None
    }
}

impl OnResize for Pty {
    fn on_resize(&mut self, _window_size: WindowSize) {}
}
