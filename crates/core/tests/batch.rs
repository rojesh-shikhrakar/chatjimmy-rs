use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
use std::sync::Arc;
use std::time::Duration;

use chatjimmy::{ChatRequest, Client, Error, Message};
use futures::StreamExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[derive(Default)]
struct Peak {
    inflight: AtomicUsize,
    peak: AtomicUsize,
}

/// Multi-connection mock. Echoes the prompt back; "slow" takes 200 ms, "bad" gets a 400,
/// everything else takes 30 ms so concurrent requests overlap measurably.
async fn server(peak: Arc<Peak>) -> Client {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        loop {
            let (sock, _) = listener.accept().await.unwrap();
            tokio::spawn(handle(sock, peak.clone()));
        }
    });
    Client::builder().base_url(&url).max_retries(0).build().unwrap()
}

async fn handle(mut sock: TcpStream, peak: Arc<Peak>) {
    loop {
        let mut buf = Vec::new();
        let mut tmp = [0u8; 4096];
        let body = loop {
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
            if buf.len() >= i + 4 + len {
                break buf[i + 4..i + 4 + len].to_vec();
            }
        };
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let prompt = v["messages"][0]["content"].as_str().unwrap().to_string();

        let now = peak.inflight.fetch_add(1, SeqCst) + 1;
        peak.peak.fetch_max(now, SeqCst);
        let delay = if prompt == "slow" { 200 } else { 30 };
        tokio::time::sleep(Duration::from_millis(delay)).await;
        peak.inflight.fetch_sub(1, SeqCst);

        let (status, text) = if prompt == "bad" {
            ("400 Bad Request", "bad".to_string())
        } else {
            ("200 OK", format!("{prompt}<|stats|>{{}}<|/stats|>"))
        };
        let resp = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\n\r\n{text}",
            text.len()
        );
        if sock.write_all(resp.as_bytes()).await.is_err() {
            return;
        }
    }
}

fn reqs(prompts: &[&str]) -> Vec<ChatRequest> {
    prompts.iter().map(|p| ChatRequest::new([Message::user(*p)])).collect()
}

#[tokio::test]
async fn chat_many_preserves_input_order() {
    let c = server(Default::default()).await;
    let out = c.chat_many(reqs(&["slow", "a", "b"]), 3).await;
    let texts: Vec<_> = out.into_iter().map(|r| r.unwrap().text).collect();
    assert_eq!(texts, ["slow", "a", "b"]);
}

#[tokio::test]
async fn chat_many_stream_is_completion_order() {
    let c = server(Default::default()).await;
    let idx: Vec<usize> = c
        .chat_many_stream(reqs(&["slow", "a", "b"]), 3)
        .map(|(i, r)| {
            r.unwrap();
            i
        })
        .collect()
        .await;
    assert_eq!(idx.len(), 3);
    assert_eq!(*idx.last().unwrap(), 0, "slow request should finish last: {idx:?}");
}

#[tokio::test]
async fn one_failure_leaves_others_intact() {
    let c = server(Default::default()).await;
    let out = c.chat_many(reqs(&["a", "bad", "c"]), 3).await;
    assert_eq!(out[0].as_ref().unwrap().text, "a");
    assert!(matches!(out[1], Err(Error::Status { code: 400, .. })));
    assert_eq!(out[2].as_ref().unwrap().text, "c");
}

#[tokio::test]
async fn concurrency_cap_respected() {
    let peak = Arc::new(Peak::default());
    let c = server(peak.clone()).await;
    let prompts: Vec<String> = (0..20).map(|i| format!("p{i}")).collect();
    let refs: Vec<&str> = prompts.iter().map(String::as_str).collect();
    let out = c.chat_many(reqs(&refs), 4).await;
    assert!(out.iter().all(|r| r.is_ok()));
    let p = peak.peak.load(SeqCst);
    assert!(p <= 4, "peak {p} exceeded cap");
    assert!(p > 1, "no overlap observed (peak {p})");
}

#[tokio::test]
async fn empty_batch_returns_empty() {
    let c = Client::new().unwrap(); // never touches the network
    assert!(c.chat_many(vec![], 4).await.is_empty());
}

#[tokio::test]
async fn zero_concurrency_clamped() {
    let c = server(Default::default()).await;
    let out = tokio::time::timeout(Duration::from_secs(5), c.chat_many(reqs(&["a", "b"]), 0))
        .await
        .expect("deadlocked");
    assert_eq!(out.len(), 2);
    assert!(out.iter().all(|r| r.is_ok()));
}
