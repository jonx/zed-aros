#[cfg(all(not(target_os = "windows"), not(target_os = "aros")))]
pub use smol::net::unix::{UnixListener, UnixStream};

// AROS has no Unix-domain sockets; async stub types (networking-stubbed build).
#[cfg(target_os = "aros")]
pub use aros_uds::{UnixListener, UnixStream};
#[cfg(target_os = "aros")]
pub mod aros_uds {
    use std::io::{Error, ErrorKind, Result};
    use std::path::Path;

    fn unsupported<T>() -> Result<T> {
        Err(Error::new(
            ErrorKind::Unsupported,
            "Unix domain sockets are not available on AROS",
        ))
    }

    #[derive(Debug)]
    pub struct UnixListener(());

    impl UnixListener {
        pub fn bind<P: AsRef<Path>>(_path: P) -> Result<Self> {
            unsupported()
        }
        pub async fn accept(&self) -> Result<(UnixStream, ())> {
            unsupported()
        }
    }

    #[derive(Debug)]
    pub struct UnixStream(());

    impl UnixStream {
        pub async fn connect<P: AsRef<Path>>(_path: P) -> Result<Self> {
            unsupported()
        }
    }

    use std::pin::Pin;
    use std::task::{Context, Poll};

    use smol::io::{AsyncRead, AsyncWrite};

    // The stub stream is never actually connected (`connect` errors), so these
    // report immediate EOF / success; they exist only so the type satisfies the
    // async I/O bounds callers require.
    impl AsyncRead for UnixStream {
        fn poll_read(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            _buf: &mut [u8],
        ) -> Poll<Result<usize>> {
            Poll::Ready(Ok(0))
        }
    }

    impl AsyncWrite for UnixStream {
        fn poll_write(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            _buf: &[u8],
        ) -> Poll<Result<usize>> {
            Poll::Ready(unsupported())
        }
        fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<()>> {
            Poll::Ready(Ok(()))
        }
        fn poll_close(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<()>> {
            Poll::Ready(Ok(()))
        }
    }
}

#[cfg(target_os = "windows")]
pub use windows::{UnixListener, UnixStream};

#[cfg(target_os = "windows")]
pub mod windows {
    use std::{
        io::Result,
        path::Path,
        pin::Pin,
        task::{Context, Poll},
    };

    use smol::{
        Async,
        io::{AsyncRead, AsyncWrite},
    };

    pub struct UnixListener(Async<crate::UnixListener>);

    impl UnixListener {
        pub fn bind<P: AsRef<Path>>(path: P) -> Result<Self> {
            Ok(UnixListener(Async::new(crate::UnixListener::bind(path)?)?))
        }

        pub async fn accept(&self) -> Result<(UnixStream, ())> {
            let (sock, _) = self.0.read_with(|listener| listener.accept()).await?;
            Ok((UnixStream(Async::new(sock)?), ()))
        }
    }

    pub struct UnixStream(Async<crate::UnixStream>);

    impl UnixStream {
        pub async fn connect<P: AsRef<Path>>(path: P) -> Result<Self> {
            Ok(UnixStream(Async::new(crate::UnixStream::connect(path)?)?))
        }
    }

    impl AsyncRead for UnixStream {
        fn poll_read(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &mut [u8],
        ) -> Poll<Result<usize>> {
            Pin::new(&mut self.0).poll_read(cx, buf)
        }
    }

    impl AsyncWrite for UnixStream {
        fn poll_write(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<Result<usize>> {
            Pin::new(&mut self.0).poll_write(cx, buf)
        }

        fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
            Pin::new(&mut self.0).poll_flush(cx)
        }

        fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
            Pin::new(&mut self.0).poll_close(cx)
        }
    }
}
