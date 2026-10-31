use std::future::Future;
use std::time::Duration;

use chatjimmy::{ChatRequest, Chunk, Client, Message};
use futures::StreamExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio::time::timeout;

fn req() -> ChatRequest {
    ChatRequest::new([Message::user("x")])
}

/// One-shot raw HTTP server: reads the request, then hands the socket to `f`.
async fn raw_server<F, Fut>(f: F) -> Client
where
    F: FnOnce(TcpStream) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send,
{
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut buf = Vec::new();
        let mut tmp = [0u8; 1024];
        let (head_end, len) = loop {
            let n = sock.read(&mut tmp).await.unwrap();
            buf.extend_from_slice(&tmp[..n]);
            if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&buf[..i]).to_lowercase();
                let len = head
                    .lines()
                    .find_map(|l| l.strip_prefix("content-length: "))
                    .map_or(0, |v| v.trim().parse().unwrap());
                break (i + 4, len);
            }
        };
        while buf.len() < head_end + len {
            let n = sock.read(&mut tmp).await.unwrap();
            buf.extend_from_slice(&tmp[..n]);
        }
        f(sock).await;
    });
    Client::builder().base_url(&url).max_retries(0).build().unwrap()
}

async fn headers(s: &mut TcpStream) {
    s.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n").await.unwrap();
}

async fn chunk(s: &mut TcpStream, data: &str) {
    s.write_all(format!("{:x}\r\n{}\r\n", data.len(), data).as_bytes()).await.unwrap();
    s.flush().await.unwrap();
}

const LONG: &str = "hello world, this is a long first chunk";

#[tokio::test]
async fn streams_text_before_stats_arrive() {
    let (tx, rx) = oneshot::channel::<()>();
    let client = raw_server(|mut s| async move {
        headers(&mut s).await;
        chunk(&mut s, LONG).await;
        let _ = rx.await; // the test only releases the rest after seeing early text
        chunk(&mut s, "<|stats|>{}<|/stats|>").await;
        s.write_all(b"0\r\n\r\n").await.unwrap();
    })
    .await;
    let mut stream = client.chat_stream(req()).await.unwrap();
    let first = timeout(Duration::from_secs(2), stream.next())
        .await
        .expect("no text before the rest of the body was sent")
        .unwrap()
        .unwrap();
    let Chunk::Text(t) = first else { panic!("expected text first") };
    assert!(!t.is_empty() && LONG.starts_with(&t));
    tx.send(()).unwrap();
    let mut text = t;
    let mut stats = None;
    while let Some(c) = stream.next().await {
        match c.unwrap() {
            Chunk::Text(t) => text += &t,
            Chunk::Done(s) => stats = Some(s),
        }
    }
    assert_eq!(text, LONG);
    assert!(stats.is_some());
}

#[tokio::test]
async fn concatenated_chunks_equal_chat_text_and_end_with_done() {
    let body = "hello world, plenty of text here<|stats|>{\"decode_tokens\":7}<|/stats|>";
    let make = || {
        raw_server(move |mut s| async move {
            headers(&mut s).await;
            chunk(&mut s, body).await;
            s.write_all(b"0\r\n\r\n").await.unwrap();
        })
    };
    let expected = make().await.chat(req()).await.unwrap().text;
    let mut stream = make().await.chat_stream(req()).await.unwrap();
    let mut text = String::new();
    let mut last = None;
    while let Some(c) = stream.next().await {
        last = Some(c.unwrap());
        if let Some(Chunk::Text(t)) = &last {
            text += t;
        }
    }
    assert_eq!(text, expected);
    assert!(matches!(last, Some(Chunk::Done(s)) if s.decode_tokens == 7));
}

#[tokio::test]
async fn mid_stream_disconnect_is_err() {
    let client = raw_server(|mut s| async move {
        headers(&mut s).await;
        chunk(&mut s, LONG).await;
        // drop without the terminating 0-chunk
    })
    .await;
    let mut stream = client.chat_stream(req()).await.unwrap();
    let mut saw_err = false;
    while let Some(c) = timeout(Duration::from_secs(2), stream.next()).await.expect("hung") {
        if c.is_err() {
            saw_err = true;
        }
    }
    assert!(saw_err, "truncated body must surface as Err");
}

#[tokio::test]
async fn early_drop_is_fine() {
    let (tx, rx) = oneshot::channel::<()>();
    let client = raw_server(|mut s| async move {
        headers(&mut s).await;
        chunk(&mut s, LONG).await;
        let _ = rx.await;
        let _ = s.write_all(b"5\r\nhello\r\n").await; // client is gone; must not matter
    })
    .await;
    let mut stream = client.chat_stream(req()).await.unwrap();
    assert!(stream.next().await.unwrap().is_ok());
    drop(stream);
    let _ = tx.send(());
}
