# chatjimmy

Unofficial async Rust client for the [chatjimmy.ai](https://chatjimmy.ai) API (Taalas HC1 inference).
The API has no authentication and may change or go offline at any time.

## Usage

```rust
use chatjimmy::{ChatRequest, Chunk, Client, Message};
use futures::StreamExt;

#[tokio::main]
async fn main() -> chatjimmy::Result<()> {
    let client = Client::new()?;

    // Default: one call, full response + stats.
    let r = client.chat(ChatRequest::new([Message::user("Explain recursion")])).await?;
    println!("{} ({} tokens)", r.text, r.stats.map_or(0, |s| s.decode_tokens));

    // Opt-in streaming.
    let mut s = client.chat_stream(ChatRequest::new([Message::user("Write a haiku")])).await?;
    while let Some(chunk) = s.next().await {
        match chunk? {
            Chunk::Text(t) => print!("{t}"),
            Chunk::Done(stats) => println!("\n{:.0} tok/s", stats.decode_rate),
        }
    }

    // Many requests, at most 16 in flight. Results in input order, one Result per item:
    let reqs: Vec<_> = (0..100).map(|i| ChatRequest::new([Message::user(format!("{i}+1="))])).collect();
    let results = client.chat_many(reqs.clone(), 16).await;

    // ...or as they finish (index, result):
    let mut done = std::pin::pin!(client.chat_many_stream(reqs, 16));
    while let Some((i, r)) = done.next().await { println!("{i}: {:?}", r.map(|r| r.text)); }
    Ok(())
}
```

`Client` is cheap to `clone()`. `concurrency` is always explicit.

## Errors

`chatjimmy::Error`: `Http` (transport/timeout/JSON), `Status { code, body }`, `EmptyResponse` (HTTP 200 with
nothing in it; the server does this when input exceeds ~6k tokens), `BadStats`, `Decode`.
Connect errors, timeouts, 429 and 5xx are retried (3 times, exponential backoff with jitter).
Streaming retries only before the first byte; a mid-stream failure arrives as `Err`.

## Tests

```
cargo test                    # mocked; never touches the network
cargo test -- --ignored       # live smoke tests against chatjimmy.ai
cargo bench                   # criterion, against a local mock (5 ms per reply)
```

## Benchmark

Local mock with a fixed 5 ms reply delay, so this measures client overhead and concurrency, not the real
service. Wall time for N requests; the Python baseline (`bench/python_baseline.py`) is `requests` + a thread
pool doing the same POST-and-read-body, best of 5.

| N | concurrency | Rust (`chat_many`) | Python (`requests`) |
|---|---|---|---|
| 128 | 1 | 887 ms (144 req/s) | 1143 ms (112 req/s) |
| 128 | 8 | 116 ms (1107 req/s) | 237 ms (540 req/s) |
| 128 | 32 | 31 ms (4129 req/s) | 287 ms (446 req/s) |

Numbers are from one machine and one run; rerun both for your own.
