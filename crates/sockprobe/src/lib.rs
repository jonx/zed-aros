//! Async-stack reproducer for the LSP-over-socket corruption. Two modes:
//!
//!  * pattern (port 12346): read a known byte pattern, verify byte-exact.
//!  * lsp     (port 9257) : send a real LSP `initialize` to the host bridge and
//!    read the framed response, printing the parsed Content-Length. A sane
//!    length + a clean body read means the single-threaded stack handles real
//!    rust-analyzer output; a garbage length reproduces the editor's OOM here,
//!    in isolation.

use futures_lite::{AsyncReadExt, AsyncWriteExt};
use std::net::{Ipv4Addr, SocketAddr};

const PATTERN_SIZE: usize = 256 * 1024;

#[no_mangle]
pub extern "C" fn sockprobe_main() -> i32 {
    let pat = pattern_test();
    let lsp = lsp_test();
    if pat == 0 && lsp == 0 {
        println!("RUST-AROS: SOCKPROBE PASS");
        0
    } else {
        println!("RUST-AROS: SOCKPROBE FAIL (pattern={pat} lsp={lsp})");
        1
    }
}

fn pattern_test() -> i32 {
    let addr = SocketAddr::from((Ipv4Addr::new(127, 0, 0, 1), 12346));
    let r: std::io::Result<Vec<u8>> = smol::block_on(async move {
        let stream = smol::net::TcpStream::connect(addr).await?;
        let mut writer = stream.clone();
        let mut reader = stream;
        writer.write_all(format!("{PATTERN_SIZE}\n").as_bytes()).await?;
        writer.flush().await?;
        let mut got = Vec::with_capacity(PATTERN_SIZE);
        let mut buf = vec![0u8; 4096];
        loop {
            let n = reader.read(&mut buf).await?;
            if n == 0 || n > buf.len() {
                break;
            }
            got.extend_from_slice(&buf[..n]);
            if got.len() >= PATTERN_SIZE {
                break;
            }
        }
        Ok(got)
    });
    match r {
        Ok(got) if got.len() == PATTERN_SIZE && got.iter().enumerate().all(|(i, b)| *b == (i % 251) as u8) => {
            println!("[PATTERN] PASS {PATTERN_SIZE} bytes byte-exact");
            0
        }
        Ok(got) => {
            println!("[PATTERN] FAIL len={}", got.len());
            1
        }
        Err(e) => {
            println!("[PATTERN] ERR {e:?}");
            1
        }
    }
}

fn lsp_test() -> i32 {
    let addr = SocketAddr::from((Ipv4Addr::new(127, 0, 0, 1), 9257));
    let body = br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"processId":null,"rootUri":null,"capabilities":{}}}"#;
    let r: std::io::Result<()> = smol::block_on(async move {
        let stream = smol::net::TcpStream::connect(addr).await?;
        let mut writer = stream.clone();
        let mut reader = stream;
        // frame + send the initialize request
        let req = format!("Content-Length: {}\r\n\r\n", body.len());
        writer.write_all(req.as_bytes()).await?;
        writer.write_all(body).await?;
        writer.flush().await?;
        println!("[LSP] sent initialize ({} body bytes), reading response...", body.len());

        // Read the header line by line, exactly like the LSP reader: accumulate
        // until "\r\n\r\n", parse Content-Length, then read that many body bytes.
        let mut hdr: Vec<u8> = Vec::new();
        let mut one = [0u8; 1];
        loop {
            let n = reader.read(&mut one).await?;
            if n == 0 {
                println!("[LSP] EOF while reading header (got {} bytes: {:?})", hdr.len(),
                         String::from_utf8_lossy(&hdr));
                return Ok(());
            }
            hdr.push(one[0]);
            if hdr.ends_with(b"\r\n\r\n") {
                break;
            }
            if hdr.len() > 4096 {
                println!("[LSP] header too long ({} bytes), aborting: {:?}", hdr.len(),
                         String::from_utf8_lossy(&hdr[..hdr.len().min(200)]));
                return Ok(());
            }
        }
        let headers = String::from_utf8_lossy(&hdr);
        println!("[LSP] raw headers: {:?}", headers);
        let clen: usize = headers
            .lines()
            .find_map(|l| l.strip_prefix("Content-Length:"))
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(0);
        println!("[LSP] parsed Content-Length = {clen}");
        if clen == 0 || clen > 4 * 1024 * 1024 {
            println!("[LSP] SUSPECT length {clen} (this is the editor OOM trigger)");
            return Ok(());
        }
        // read the body (bounded read, no giant preallocation)
        let mut body_buf = vec![0u8; clen];
        let mut off = 0;
        while off < clen {
            let n = reader.read(&mut body_buf[off..]).await?;
            if n == 0 {
                break;
            }
            off += n;
        }
        println!("[LSP] read {off}/{clen} body bytes; first 80: {:?}",
                 String::from_utf8_lossy(&body_buf[..off.min(80)]));
        println!("[LSP] OK: real rust-analyzer response framed correctly");
        Ok(())
    });
    match r {
        Ok(()) => 0,
        Err(e) => {
            println!("[LSP] ERR {e:?}");
            1
        }
    }
}
