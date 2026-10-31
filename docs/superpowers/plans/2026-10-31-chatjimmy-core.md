# chatjimmy-core Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** An async Rust client crate for chatjimmy.ai with non-streaming `chat` as the default, opt-in real streaming, and bounded-concurrency batch calls.

**Architecture:** One pure incremental `Parser` (bytes in, text and stats out) is shared by `chat` and `chat_stream`. `Client` wraps `Arc<reqwest::Client>` plus config; every HTTP call goes through one `send_with_retry`. `chat_many_stream` is `buffer_unordered` over `chat`, and `chat_many` collects it by index.

**Tech Stack:** Rust 2021, tokio, reqwest (rustls, stream, json), futures, serde/serde_json, thiserror, fastrand (jitter); dev: wiremock, criterion.

**Spec:** `docs/superpowers/specs/2026-10-31-chatjimmy-core-design.md`

## Global Constraints

- Crate name `chatjimmy`, path `crates/core/`, in a workspace at the repo root (`members = ["crates/core"]` for now).
- Defaults: model `"llama3.1-8B"`, `top_k` 8, base URL `https://chatjimmy.ai`, connect timeout 5 s, read timeout 30 s per chunk, no total timeout, 3 retries, backoff starting at 200 ms with jitter.
- Stats marker: `<|stats|>{json}<|/stats|>` at the end of the body.
- Request body: `{"messages":[{role,content}], "chatOptions":{"selectedModel","systemPrompt","topK"}, "attachment": null | {name,size,content}}`.
- Health JSON keys: `status, nextjs, backend, backendStatus, backendDetails, timestamp`. Models JSON: `{"data":[{id, object, created, owned_by}]}`.
- Retry: connect errors, timeouts, 429, 5xx. Never retry other 4xx or `EmptyResponse`. Streaming retries only before the first byte.
- `User-Agent` is `chatjimmy-rust/<CARGO_PKG_VERSION>`; `Content-Type` is set only on the POST.
- `concurrency` is a required argument in the library.
- CI tests never touch the real service; the live test is `#[ignore]`.

## Review Focus

Each line has its test in the owning task.

1. `chat_many(vec![], n)` returns an empty `Vec` and does not hang (Task 6).
2. `concurrency = 0` is treated as 1, not a panic or deadlock (Task 6).
3. Server drops the connection mid-stream: the stream yields `Err`, not a hang or truncated `Ok` (Task 5).
4. Dropping a `ChatStream` early stops reading and does not panic (Task 5).
5. A 200 on `health`/`models` with a non-JSON body gives `Err`, not a panic (Task 3).

## File Structure

```
Cargo.toml                      workspace
crates/core/Cargo.toml
crates/core/src/lib.rs          re-exports only
crates/core/src/types.rs        Message, Attachment, Stats, ChatRequest, ChatResponse, Chunk, Model, HealthStatus
crates/core/src/error.rs        Error enum
crates/core/src/parser.rs       Parser (pure)
crates/core/src/client.rs       Client, ClientBuilder, send_with_retry, chat/ask/health/models
crates/core/src/stream.rs       chat_stream
crates/core/src/batch.rs        chat_many_stream, chat_many
crates/core/tests/              wiremock integration tests (one file per task 3-6)
crates/core/benches/overhead.rs criterion
```

### Task 1: Workspace, types, errors

**Files:** Create `Cargo.toml`, `crates/core/Cargo.toml`, `src/lib.rs`, `src/types.rs`, `src/error.rs`; Test `src/types.rs` (`#[cfg(test)]`).

**Interfaces:**
- Produces:
  - `Message { role: String, content: String }` with `Message::user(impl Into<String>)`, `::assistant(..)`, `::system(..)`; `Serialize`.
  - `Attachment { name: String, size: u64, content: String }`; `Serialize`.
  - `Stats` (fields as in the Python `Stats`: `created_at f64, done bool, done_reason String, total_duration f64, ttft f64, prefill_tokens u64, prefill_rate f64, decode_tokens u64, decode_rate f64, total_tokens u64, total_time f64, roundtrip_time f64, topk u64, status u64, reason String`); `Deserialize + Default + Clone + Debug`, `#[serde(default)]`.
  - `ChatRequest::new(impl IntoIterator<Item = Message>) -> Self` with builder methods `.model(&str)`, `.system_prompt(&str)`, `.top_k(u32)`, `.attachment(Attachment)`, and `fn body(&self) -> serde_json::Value` producing the request-body shape above.
  - `ChatResponse { text: String, stats: Option<Stats> }`; `enum Chunk { Text(String), Done(Stats) }`.
  - `Model`, `HealthStatus` (with `fn healthy(&self) -> bool` = `status == "ok" && backend == "healthy"`), both `Deserialize` with `#[serde(rename_all = "camelCase", default)]` where needed (`owned_by` stays snake_case on `Model`).
  - `Error` exactly as in the spec, plus `#[from]` for `reqwest::Error`; `pub type Result<T> = std::result::Result<T, Error>`.

- [ ] **Step 1: Write failing tests in `types.rs`:** `request_body_defaults` asserts `ChatRequest::new([Message::user("hi")]).body()` equals `{"messages":[{"role":"user","content":"hi"}],"chatOptions":{"selectedModel":"llama3.1-8B","systemPrompt":"","topK":8},"attachment":null}`; `stats_ignores_unknown_and_defaults_missing` parses `{"decode_tokens":5,"bogus":1}` into `Stats` with `decode_tokens == 5` and everything else default; `health_healthy` parses `{"status":"ok","backend":"healthy","backendStatus":200}`.
- [ ] **Step 2:** Run `cargo test -p chatjimmy` — expect compile failure (types undefined).
- [ ] **Step 3: Implement** the files above. `lib.rs` is `mod` declarations plus `pub use`. `Error` variants and messages per the spec.
- [ ] **Step 4:** Run `cargo test -p chatjimmy` — expect 3 passed.
- [ ] **Step 5: Commit** `feat: workspace, types, errors`.

### Task 2: Incremental parser

**Files:** Create `src/parser.rs`; Test: same file.

**Interfaces:**
- Consumes: `Stats`, `Error` (Task 1).
- Produces: `pub(crate) struct Parser`; `Parser::new() -> Self`; `fn push(&mut self, bytes: &[u8]) -> Result<String>` returns text safe to emit now; `fn finish(self) -> Result<(String, Option<Stats>)>` returns remaining text and parsed stats. Marker is `<|stats|>` … `<|/stats|>`.

Rules: bytes decode through a held buffer so a char split across pushes is completed, never emitted partially; invalid UTF-8 that cannot be completed gives `Error::Decode` (also a truncated char at `finish`). `push` holds back the last 8 bytes of text (`len("<|stats|>") - 1`) in case they begin a marker; once `<|stats|>` is seen, nothing after it is emitted. `finish` parses the JSON between the markers (a missing end marker just means the JSON is whatever follows; truncated JSON surfaces as `BadStats`). If the total emitted text is empty and there are no stats, `finish` returns `Error::EmptyResponse`.

- [ ] **Step 1: Write failing tests:** `text_then_stats` (`"hello<|stats|>{\"decode_tokens\":3}<|/stats|>"` gives text `"hello"`, `decode_tokens == 3`); `split_at_every_byte_offset` (same body fed as two pushes at each offset 0..=len, always the same final text and stats); `multibyte_split` (body `"é😀"` plus stats split inside each char gives the exact text); `no_stats_block` (`"plain"` gives text `"plain"`, `None`); `empty_body_is_empty_response`; `malformed_stats_is_bad_stats` (`<|stats|>{nope<|/stats|>`); `unknown_stats_fields_ignored`; `partial_marker_not_emitted` (push `"abc<|sta"` returns no `"<|sta"`).
- [ ] **Step 2:** `cargo test -p chatjimmy parser` — expect FAIL (not defined).
- [ ] **Step 3: Implement** `Parser` per the rules; use `std::str::from_utf8` with `Utf8Error::valid_up_to()`/`error_len()` to distinguish incomplete from invalid.
- [ ] **Step 4:** `cargo test -p chatjimmy parser` — expect 8 passed.
- [ ] **Step 5: Commit** `feat: incremental stats parser`.

### Task 3: Client, `chat`, `ask`, `health`, `models`

**Files:** Create `src/client.rs`; Modify `lib.rs`; Test `tests/basic.rs` (wiremock).

**Interfaces:**
- Consumes: `Parser`, types, `Error`.
- Produces: `Client` (`Clone`, wraps `Arc` inner); `Client::builder() -> ClientBuilder` with `.base_url(&str)`, `.connect_timeout(Duration)`, `.read_timeout(Duration)`, `.timeout(Duration)`, `.max_retries(u32)`, `.build() -> Result<Client>`; `Client::new() -> Result<Client>` = defaults; `async fn chat(&self, req: ChatRequest) -> Result<ChatResponse>`; `async fn ask(&self, prompt: &str) -> Result<String>`; `async fn health(&self) -> Result<HealthStatus>`; `async fn models(&self) -> Result<Vec<Model>>`. Internal `async fn post_chat(&self, req: &ChatRequest) -> Result<reqwest::Response>` used by Task 5. Non-2xx gives `Error::Status { code, body }`.

- [ ] **Step 1: Write failing tests (wiremock):** `chat_returns_text_and_stats` (mock returns `hi<|stats|>{"decode_tokens":2}<|/stats|>`; assert `text == "hi"`, `stats.unwrap().decode_tokens == 2`, and the request body matched `req.body()`); `chat_sends_content_type_and_user_agent` (UA starts with `chatjimmy-rust/`); `get_requests_have_no_content_type`; `ask_returns_text`; `health_parses`; `models_parses`; `status_error_carries_body` (400 with body `"bad"` gives `Error::Status{code:400, body:"bad"}`); `empty_200_is_empty_response`; `health_non_json_is_error` (Review Focus 5; 200 with body `"<html>"` gives `Err`).
- [ ] **Step 2:** `cargo test -p chatjimmy --test basic` — expect FAIL.
- [ ] **Step 3: Implement** `client.rs`. `chat` reads the full body with `bytes_stream()`, feeds `Parser`, and builds the response. The builder maps `read_timeout` to `reqwest::ClientBuilder::read_timeout`, and `max_retries` is stored (used in Task 4). Set `Content-Type` via `.json()` on the POST only.
- [ ] **Step 4:** `cargo test -p chatjimmy --test basic` — expect 9 passed.
- [ ] **Step 5: Commit** `feat: client with chat, ask, health, models`.

### Task 4: Retries

**Files:** Modify `src/client.rs`; Test `tests/retry.rs`.

**Interfaces:**
- Consumes: Task 3 `Client`. Produces: `async fn send_with_retry(&self, build: impl Fn() -> reqwest::RequestBuilder) -> Result<reqwest::Response>`; `post_chat`, `health`, `models` all route through it.

Backoff: `200ms * 2^attempt` plus `fastrand` jitter up to 50%. A test-only knob, `ClientBuilder::retry_base_delay(Duration)` (public, documented), keeps tests fast.

- [ ] **Step 1: Write failing tests:** `retries_503_then_succeeds` (503, 503, then 200: result `Ok`, mock saw 3 calls); `gives_up_after_max_retries` (always 503 with `max_retries(2)`: `Err(Status{503,..})`, 3 calls); `no_retry_on_400` (1 call); `retries_429`; `no_retry_on_empty_response` (1 call, `Err(EmptyResponse)`).
- [ ] **Step 2:** `cargo test -p chatjimmy --test retry` — expect FAIL.
- [ ] **Step 3: Implement** `send_with_retry`; retry on `reqwest::Error` that `is_connect() || is_timeout()`, and on status 429 or 5xx.
- [ ] **Step 4:** run `cargo test -p chatjimmy` — all pass.
- [ ] **Step 5: Commit** `feat: retries with backoff`.

### Task 5: Streaming

**Files:** Create `src/stream.rs`; Modify `lib.rs`; Test `tests/stream.rs`.

**Interfaces:**
- Consumes: `Client::post_chat`, `Parser`.
- Produces: `async fn chat_stream(&self, req: ChatRequest) -> Result<impl Stream<Item = Result<Chunk>>>` (returned as `Pin<Box<dyn Stream<Item = Result<Chunk>> + Send>>`). Yields `Chunk::Text` for each non-empty `push` result and a final `Chunk::Done(stats)`; if there are no stats, the stream just ends after the remaining text. Errors from `finish` (`EmptyResponse`, `BadStats`) are yielded as the last item, and a transport error mid-body is yielded as `Err` and ends the stream. No retry once the first byte has been read.

- [ ] **Step 1: Write failing tests:** `streams_text_before_stats_arrive` (hand-rolled `tokio` TCP server sends `"hel"`, sleeps, sends `"lo<|stats|>{}<|/stats|>"`; assert the first `Text` item arrives before the second write, e.g. by timing or by a channel the server signals); `concatenated_chunks_equal_chat_text`; `ends_with_done_stats`; `mid_stream_disconnect_is_err` (server writes a partial chunked body, then resets: stream yields `Err`; Review Focus 3); `early_drop_is_fine` (drop after the first item, server doesn't panic, test completes; Review Focus 4).
- [ ] **Step 2:** `cargo test -p chatjimmy --test stream` — expect FAIL.
- [ ] **Step 3: Implement** with `futures::stream::try_unfold` or `async_stream`-free manual state over `bytes_stream()`.
- [ ] **Step 4:** run `cargo test -p chatjimmy` — all pass.
- [ ] **Step 5: Commit** `feat: opt-in streaming chat`.

### Task 6: Batch

**Files:** Create `src/batch.rs`; Modify `lib.rs`; Test `tests/batch.rs`.

**Interfaces:**
- Consumes: `Client::chat`.
- Produces: `fn chat_many_stream(&self, reqs: Vec<ChatRequest>, concurrency: usize) -> impl Stream<Item = (usize, Result<ChatResponse>)>`; `async fn chat_many(&self, reqs: Vec<ChatRequest>, concurrency: usize) -> Vec<Result<ChatResponse>>`. `concurrency` is clamped to at least 1.

- [ ] **Step 1: Write failing tests:** `chat_many_preserves_input_order` (first request delayed 200 ms, others instant: results `[0]` still maps to request 0's text); `chat_many_stream_is_completion_order` (the delayed index arrives last); `one_failure_leaves_others_intact` (the middle request gets a 400, the others `Ok`); `concurrency_cap_respected` (mock records peak in-flight with an atomic counter, 20 requests at `concurrency=4`: peak ≤ 4 and > 1); `empty_batch_returns_empty` (Review Focus 1); `zero_concurrency_clamped` (Review Focus 2; `concurrency=0` completes with all results).
- [ ] **Step 2:** `cargo test -p chatjimmy --test batch` — expect FAIL.
- [ ] **Step 3: Implement** `stream::iter(reqs).enumerate().map(|(i, r)| async move { (i, self.chat(r).await) }).buffer_unordered(n.max(1))`; `chat_many` collects into `Vec<Option<_>>` by index then unwraps.
- [ ] **Step 4:** run `cargo test -p chatjimmy` — all pass.
- [ ] **Step 5: Commit** `feat: concurrent batch chat`.

### Task 7: Live smoke test, benchmark, README

**Files:** Create `tests/live.rs`, `benches/overhead.rs`, `bench/python_baseline.py`, `README.md`.

**Interfaces:** Consumes the whole public API.

- [ ] **Step 1: Write `tests/live.rs`:** `#[ignore] async fn live_ask` asserts `Client::new()?.ask("Say hi").await?` is non-empty, and `#[ignore] async fn live_batch` runs 8 prompts at concurrency 4 and asserts all `Ok`.
- [ ] **Step 2: Write `benches/overhead.rs`:** criterion group against a local wiremock/hyper mock, N ∈ {1, 16, 128} × concurrency ∈ {1, 8, 32}, using `chat_many`. `bench/python_baseline.py` runs the same workload through the old `requests` client against the same mock URL (URL via argv) with a `ThreadPoolExecutor`, printing requests/sec.
- [ ] **Step 3:** Run `cargo bench -p chatjimmy` and the Python baseline against the same mock; record both sets of numbers in `README.md` alongside install, `chat` / `chat_stream` / `chat_many` usage, error handling, and the live-test command.
- [ ] **Step 4:** Run `cargo test -p chatjimmy` (all pass, live ignored) and `cargo test -p chatjimmy -- --ignored` (live passes if the service is up; report honestly if it's down).
- [ ] **Step 5: Commit** `feat: live tests, benchmark, README`.
