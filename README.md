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

## CLI

```
cargo install --path crates/cli     # installs `chatjimmy`

chatjimmy ask "Explain recursion"          # whole reply at once
chatjimmy ask "Write a haiku" --stream     # print as it arrives
echo "hi" | chatjimmy ask -                # prompt from stdin
chatjimmy ask "hi" --stats                 # stats JSON on stderr
chatjimmy chat                             # interactive multi-turn chat
chatjimmy batch prompts.txt -c 16          # many prompts, JSONL out (stdin if no file)
```

Shared options: `--model`, `--system`, `--top-k`, `--base-url`.

### Chat

`chatjimmy chat` reads one message per line from stdin and streams each reply to stdout. Every request carries
the full conversation history. `/reset` clears it, `/exit` (or `/quit`, or EOF) quits. The `> ` prompt goes to
stderr and only when stdin is a terminal, so piping works (`printf 'hi\nand again?\n' | chatjimmy chat`).
A failed turn prints `chatjimmy: error: ...` to stderr, is dropped from the history, and the session continues.
Set `--system`, `--model`, `--top-k` up front; history is in memory only and grows until `/reset`
(very long chats will hit the ~6k-token input limit).

### Batch

Each non-blank input line is a plain prompt, or a JSON object with a string `prompt` and optional `id`,
`system_prompt`, `model`, `top_k` (these override the shared options). Detection is per line, so a plain prompt
that is itself a JSON object with a `prompt` field is treated as a request.

One JSON line per request is written to stdout as each finishes (completion order):

```
{"index":0,"id":"x","ok":true,"text":"...","stats":{...}}
{"index":1,"ok":false,"error":"..."}
```

`index` is the 0-based position among non-blank input lines; `id` (a string or number) is echoed unchanged and appears only if the request had one, including on error records. A leading UTF-8 BOM is ignored.
Exit codes: `0` all succeeded, `2` some failed (including unparseable lines), `1` fatal (unreadable file, bad
arguments). `ask` exits `0` or `1`. Errors go to stderr as `chatjimmy: error: ...`. The whole input is read into
memory before the run starts.
