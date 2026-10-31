//! Local stand-in for chatjimmy.ai used by the benchmarks.
//! `cargo run --release --example mock_server -- 127.0.0.1:8099`
#![allow(dead_code)]
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const BODY: &str = "hello world, this is a canned reply<|stats|>{\"decode_tokens\":7}<|/stats|>";

/// Serves forever on `listener`, replying after `delay` to every POST.
pub async fn serve(listener: TcpListener, delay: Duration) {
    loop {
        let (sock, _) = listener.accept().await.unwrap();
        tokio::spawn(handle(sock, delay));
    }
}

async fn handle(mut sock: TcpStream, delay: Duration) {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    loop {
        let Ok(n) = sock.read(&mut tmp).await else { return };
        if n == 0 {
            return;
        }
        buf.extend_from_slice(&tmp[..n]);
        let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") else { continue };
        let head = String::from_utf8_lossy(&buf[..i]).to_lowercase();
        let len: usize = head
            .lines()
            .find_map(|l| l.strip_prefix("content-length: "))
            .map_or(0, |v| v.trim().parse().unwrap());
        if buf.len() < i + 4 + len {
            continue;
        }
        buf.drain(..i + 4 + len);
        tokio::time::sleep(delay).await;
        let resp = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{BODY}", BODY.len());
        if sock.write_all(resp.as_bytes()).await.is_err() {
            return;
        }
    }
}

#[tokio::main]
async fn main() {
    let addr = std::env::args().nth(1).unwrap_or_else(|| "127.0.0.1:8099".into());
    let listener = TcpListener::bind(&addr).await.unwrap();
    println!("mock listening on http://{addr}");
    serve(listener, Duration::from_millis(5)).await;
}
