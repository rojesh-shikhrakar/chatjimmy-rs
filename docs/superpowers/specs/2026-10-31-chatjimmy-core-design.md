# chatjimmy-core: Rust rewrite, sub-project 1

## Context

Rewrite of the Python wrapper for the unofficial chatjimmy.ai API (`/api/chat`, `/api/models`, `/api/health`) in Rust. The full project has three sub-projects, each with its own spec, plan and build cycle:

1. `chatjimmy-core`: async library crate (this spec).
2. `chatjimmy-cli`: binary with `--stream` and a batch mode (JSONL out).
3. `chatjimmy-py`: PyO3 bindings built with maturin. Async-first, not drop-in compatible with the old Python package.

Layout is one Cargo workspace in this repo. The Python source in `src/chatjimmy/` is eventually replaced by the maturin-built package of the same name.

```
Cargo.toml          (workspace)
crates/core/        chatjimmy        (lib)
crates/cli/         chatjimmy-cli    (bin)
crates/py/          chatjimmy-py     (pyo3)
```

## Goals

- Non-streaming is the default. Streaming is opt-in.
- Real streaming (the Python `chat_stream` buffered the whole body).
- Fast concurrent requests with a bounded concurrency limit.
- Fix the Python issues: silent empty response, silent stats failure, UTF-8 corruption across chunk boundaries, `Content-Type` on GET, hardcoded `User-Agent` version, single timeout.

## Approach

`tokio` + `reqwest` + `futures`; concurrency via `buffer_unordered`. Rejected: hand-rolled `spawn` + `Semaphore` + `mpsc` (no need for per-task priorities/cancellation), blocking `ureq` + thread pool (worse for many in-flight requests and for PyO3 async).

## Public API

```rust
let client = Client::builder()
    .base_url(..).connect_timeout(..).read_timeout(..)
    .max_retries(3).build()?;

let req = ChatRequest::new([Message::user("hi")])   // model="llama3.1-8B", top_k=8 defaults
    .system_prompt("..").attachment(..);

client.ask("prompt").await?;                  // -> String
client.chat(req).await?;                      // -> ChatResponse { text, stats }  (default)
client.chat_stream(req).await?;               // -> Stream<Item = Result<Chunk>>  (opt-in)
client.chat_many_stream(reqs, concurrency);   // -> Stream<Item = (usize, Result<ChatResponse>)>, completion order
client.chat_many(reqs, concurrency).await;    // -> Vec<Result<ChatResponse>>, input order, built on chat_many_stream
client.health().await?; client.models().await?;
```

- `Chunk` is `enum { Text(String), Done(Stats) }`; the stream carries its own stats.
- `Client` wraps an `Arc`; `clone()` is cheap and it is shared across tasks.
- `User-Agent` derives from `CARGO_PKG_VERSION`. `Content-Type` is set only on the POST.
- `concurrency` is a required argument in the library (no hidden default). The CLI will default to 16.

## Streaming and parsing

- The server appends `<|stats|>{json}<|/stats|>` to the end of the text body.
- The body is read as raw bytes and decoded with a stateful UTF-8 decoder; a multi-byte character split across network chunks is held until complete.
- `chat_stream` holds back the last `len("<|stats|>") - 1` bytes of unemitted text so a partial marker is never emitted as text. At end of body, the held-back tail is parsed as the stats block and yielded as `Chunk::Done(Stats)`.
- `chat()` uses the same parser over the whole body: one parsing path for both modes.
- `Stats` derives `Deserialize` with `#[serde(default)]`; unknown fields are ignored.

## Errors

```rust
enum Error {
    Http(reqwest::Error),            // connect/timeout/transport
    Status { code: u16, body: String },
    EmptyResponse,                   // 200 with no text and no stats (over-long input, ~6k tokens)
    BadStats(serde_json::Error),     // stats block present but unparseable (hard error)
    Decode,                          // unrecoverable invalid UTF-8
}
```

Implemented with `thiserror`.

## Retries

- Retried: connect errors, timeouts, 429, 5xx. Exponential backoff with jitter, starting at 200 ms, default 3 retries.
- Not retried: other 4xx, `EmptyResponse` (deterministic for over-long input).
- Streaming: retry only before the first byte. A mid-stream failure surfaces as `Err`; restarting would duplicate output.
- Batches: each request retries independently inside its own task.

## Timeouts

- `connect_timeout` default 5 s.
- `read_timeout` default 30 s, per chunk (not whole request), so long generations work.
- No total timeout by default; `.timeout(..)` sets one.

## Concurrency

`chat_many_stream` is `stream::iter(reqs).enumerate().map(..).buffer_unordered(n)`, yielding `(index, Result)` in completion order. `chat_many` collects into a `Vec<Option<_>>` by index. Failures are per item; one failed request never loses the others.

## Testing

- Parser unit tests (no network): trailing stats; stats block split at every byte offset; multi-byte char (`é`, emoji) split across chunks; no stats block; empty body gives `EmptyResponse`; malformed stats gives `BadStats`; unknown stats fields ignored.
- `wiremock` integration tests: `chat`, `chat_stream`, `health`, `models`; retry on 503 then success; no retry on 400; `chat_stream` emits text before the stats block arrives (slow chunked mock; regression for the Python bug); `chat_many` input order despite a delayed first request; `chat_many_stream` completion order; one failure in a batch leaves the rest intact; the mock records peak in-flight requests and the test asserts it never exceeds `n`.
- One live smoke test against chatjimmy.ai, `#[ignore]`, run with `cargo test -- --ignored`. CI never depends on the public demo.

## Benchmark

`criterion` against a local mock server (never the real service). Measures client overhead at N = 1, 16, 128 requests and concurrency 1, 8, 32: requests/sec and p50/p99 latency. A small script runs the same workload through the old `requests`-based client against the same mock so the README reports measured numbers.

## Out of scope

CLI and Python bindings (sub-projects 2 and 3), a blocking wrapper, rate-limit auto-tuning, proxy/TLS configuration beyond `reqwest` defaults, publishing to crates.io.
