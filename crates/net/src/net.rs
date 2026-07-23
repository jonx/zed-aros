pub mod async_net;
#[cfg(target_os = "windows")]
pub mod listener;
#[cfg(target_os = "windows")]
pub mod socket;
#[cfg(target_os = "windows")]
pub mod stream;
#[cfg(target_os = "windows")]
mod util;

#[cfg(target_os = "windows")]
pub use listener::*;
#[cfg(target_os = "windows")]
pub use socket::*;
#[cfg(all(not(target_os = "windows"), not(target_os = "aros")))]
pub use std::os::unix::net::{UnixListener, UnixStream};
#[cfg(target_os = "windows")]
pub use stream::*;

// AROS has no Unix-domain sockets; provide stub types so the abstraction
// compiles (this is a networking-stubbed build). Any use returns an error.
#[cfg(target_os = "aros")]
pub use aros_uds::{UnixListener, UnixStream};
#[cfg(target_os = "aros")]
mod aros_uds {
    use std::io::{self, Read, Write};
    use std::path::Path;

    fn unsupported<T>() -> io::Result<T> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Unix domain sockets are not available on AROS",
        ))
    }

    #[derive(Debug)]
    pub struct UnixListener(());

    impl UnixListener {
        pub fn bind<P: AsRef<Path>>(_path: P) -> io::Result<UnixListener> {
            unsupported()
        }
        pub fn accept(&self) -> io::Result<(UnixStream, ())> {
            unsupported()
        }
        pub fn set_nonblocking(&self, _nonblocking: bool) -> io::Result<()> {
            unsupported()
        }
    }

    #[derive(Debug)]
    pub struct UnixStream(());

    impl UnixStream {
        pub fn connect<P: AsRef<Path>>(_path: P) -> io::Result<UnixStream> {
            unsupported()
        }
        pub fn set_nonblocking(&self, _nonblocking: bool) -> io::Result<()> {
            unsupported()
        }
    }

    impl Read for UnixStream {
        fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
            unsupported()
        }
    }

    impl Write for UnixStream {
        fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
            unsupported()
        }
        fn flush(&mut self) -> io::Result<()> {
            unsupported()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};

    use smol::io::{AsyncReadExt, AsyncWriteExt};

    const SERVER_MESSAGE: &str = "Connection closed";
    const CLIENT_MESSAGE: &str = "Hello, server!";
    const BUFFER_SIZE: usize = 32;

    #[test]
    fn test_windows_listener() -> std::io::Result<()> {
        use crate::{UnixListener, UnixStream};

        let temp = tempfile::tempdir()?;
        let socket = temp.path().join("socket.sock");
        let listener = UnixListener::bind(&socket)?;

        // Server
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();

            // Read data from the client
            let mut buffer = [0; BUFFER_SIZE];
            let bytes_read = stream.read(&mut buffer).unwrap();
            let string = String::from_utf8_lossy(&buffer[..bytes_read]);
            assert_eq!(string, CLIENT_MESSAGE);

            // Send a message back to the client
            stream.write_all(SERVER_MESSAGE.as_bytes()).unwrap();
        });

        // Client
        let mut client = UnixStream::connect(&socket)?;

        // Send data to the server
        client.write_all(CLIENT_MESSAGE.as_bytes())?;
        let mut buffer = [0; BUFFER_SIZE];

        // Read the response from the server
        let bytes_read = client.read(&mut buffer)?;
        let string = String::from_utf8_lossy(&buffer[..bytes_read]);
        assert_eq!(string, SERVER_MESSAGE);
        client.flush()?;

        server.join().unwrap();
        Ok(())
    }

    #[test]
    fn test_unix_listener() -> std::io::Result<()> {
        use crate::async_net::{UnixListener, UnixStream};

        smol::block_on(async {
            let temp = tempfile::tempdir()?;
            let socket = temp.path().join("socket.sock");
            let listener = UnixListener::bind(&socket)?;

            // Server
            let server = smol::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();

                // Read data from the client
                let mut buffer = [0; BUFFER_SIZE];
                let bytes_read = stream.read(&mut buffer).await.unwrap();
                let string = String::from_utf8_lossy(&buffer[..bytes_read]);
                assert_eq!(string, CLIENT_MESSAGE);

                // Send a message back to the client
                stream.write_all(SERVER_MESSAGE.as_bytes()).await.unwrap();
            });

            // Client
            let mut client = UnixStream::connect(&socket).await?;
            client.write_all(CLIENT_MESSAGE.as_bytes()).await?;

            // Read the response from the server
            let mut buffer = [0; BUFFER_SIZE];
            let bytes_read = client.read(&mut buffer).await?;
            let string = String::from_utf8_lossy(&buffer[..bytes_read]);
            assert_eq!(string, "Connection closed");
            client.flush().await?;

            server.await;
            Ok(())
        })
    }
}
