//! LSP reader reproducer using the EXACT primitives input_handler.rs uses:
//! smol BufReader + AsyncBufReadExt::read_until(b'\n') for headers, and
//! read_exact for the body. Talks to the real host bridge (127.0.0.1:9257).
//! Single-threaded first: if this loses frame sync / runs read_until away, the
//! bug is in those primitives on the AROS async stream, not raw read().

use futures_lite::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};
use smol::io::BufReader;
use std::net::{Ipv4Addr, SocketAddr};

const CONTENT_LEN: &str = "Content-Length: ";
const HDR_END: &[u8; 4] = b"\r\n\r\n";
const CAP: usize = 8 * 1024 * 1024; // reproducer self-guard; real bug shows as a big printed len

async fn read_headers<R: futures_lite::AsyncBufRead + Unpin>(
    r: &mut R,
    buf: &mut Vec<u8>,
) -> std::io::Result<bool> {
    loop {
        if buf.len() >= 4 && &buf[buf.len() - 4..] == HDR_END {
            return Ok(true);
        }
        if buf.len() > CAP {
            println!("[LSP] read_headers RUNAWAY at {} bytes; first 300: {:?}",
                     buf.len(), String::from_utf8_lossy(&buf[..300.min(buf.len())]));
            return Ok(false);
        }
        let n = r.read_until(b'\n', buf).await?;
        if n == 0 {
            println!("[LSP] EOF in headers ({} bytes so far)", buf.len());
            return Ok(false);
        }
    }
}

#[no_mangle]
pub extern "C" fn sockprobe_main() -> i32 {
    let addr = SocketAddr::from((Ipv4Addr::new(127, 0, 0, 1), 9257));
    let rc: std::io::Result<i32> = smol::block_on(async move {
        let stream = smol::net::TcpStream::connect(addr).await?;
        let mut writer = stream.clone();
        let mut reader = BufReader::new(stream);

        // Send initialize (id 1), then initialized + didOpen -- same traffic the editor sends.
        for body in [
            &br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"processId":null,"rootUri":null,"capabilities":{}}}"#[..],
            &br#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#[..],
            &br#"{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":"file:///tmp/x.rs","languageId":"rust","version":1,"text":"fn main(){}\n"}}}"#[..],
        ] {
            writer.write_all(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes()).await?;
            writer.write_all(body).await?;
        }
        writer.flush().await?;
        println!("[LSP] requests sent; reading frames with BufReader/read_until/read_exact");

        let mut buffer = Vec::new();
        let mut frames = 0;
        loop {
            buffer.clear();
            if !read_headers(&mut reader, &mut buffer).await? {
                println!("[LSP] header read failed at frame {frames}");
                return Ok(1);
            }
            let headers = String::from_utf8_lossy(&buffer).to_string();
            let message_len: usize = headers
                .split('\n')
                .find(|l| l.starts_with(CONTENT_LEN))
                .and_then(|l| l.strip_prefix(CONTENT_LEN))
                .and_then(|v| v.trim_end().parse().ok())
                .unwrap_or(usize::MAX);
            println!("[LSP] frame {frames}: Content-Length={message_len}");
            if message_len == usize::MAX || message_len > CAP {
                println!("[LSP] SUSPECT len {message_len}; headers={headers:?}");
                return Ok(2);
            }
            buffer.resize(message_len, 0);
            reader.read_exact(&mut buffer).await?;
            frames += 1;
            if frames >= 3 {
                println!("[LSP] read {frames} frames cleanly via editor primitives");
                return Ok(0);
            }
        }
    });
    match rc {
        Ok(0) => { println!("RUST-AROS: SOCKPROBE PASS"); 0 }
        Ok(c) => { println!("RUST-AROS: SOCKPROBE FAIL ({c})"); c }
        Err(e) => { println!("[LSP] ERR {e:?}"); println!("RUST-AROS: SOCKPROBE FAIL"); 3 }
    }
}
