//! Hits the real chatjimmy.ai. Run with `cargo test -- --ignored`.
use chatjimmy::{ChatRequest, Client, Message};

#[tokio::test]
#[ignore]
async fn live_ask() {
    let text = Client::new().unwrap().ask("Say hi").await.unwrap();
    assert!(!text.is_empty());
}

#[tokio::test]
#[ignore]
async fn live_batch() {
    let reqs: Vec<_> = (0..8)
        .map(|i| ChatRequest::new([Message::user(format!("Say the number {i}"))]))
        .collect();
    let out = Client::new().unwrap().chat_many(reqs, 4).await;
    assert!(out.iter().all(|r| r.is_ok()), "{out:?}");
}
