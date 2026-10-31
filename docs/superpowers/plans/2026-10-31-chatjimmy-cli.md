# chatjimmy-cli Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A `chatjimmy` binary with `ask` (plain or `--stream`) and `batch` (JSONL in/out, concurrent) subcommands on top of `chatjimmy-core`.

**Architecture:** Pure, unit-tested functions (`parse_line`, `Record`, `run_batch` over a generic writer) behind a thin `clap` main. Process-level tests run the built binary against a `wiremock` server.

**Tech Stack:** Rust 2021, clap (derive), tokio, serde/serde_json, futures; dev: wiremock.

**Spec:** `docs/superpowers/specs/2026-10-31-chatjimmy-cli-design.md`

## Global Constraints

- Package `chatjimmy-cli` at `crates/cli/`, binary name `chatjimmy`; add `"crates/cli"` to the workspace `members`.
- Commands: `ask [PROMPT] [--stream] [--stats]`, `batch [FILE] [-c/--concurrency 16]`; shared `--model`, `--system`, `--top-k`, `--base-url`. Defaults for model/top-k are the core's (`llama3.1-8B`, 8).
- Batch input line = JSON object with a string `prompt` (optional `id`, `system_prompt`, `model`, `top_k`) or else a plain prompt; blank lines skipped; non-string `prompt` is an error record. Per-line fields override shared options.
- Batch output: one JSON line per request, flushed as each finishes, completion order: `{"index","id"?,"ok":true,"text","stats"}` or `{"index","id"?,"ok":false,"error"}`. `index` = 0-based among non-blank input lines.
- Exit codes: `ask` 0/1; `batch` 0 all ok, 2 any failed (including unparseable lines), 1 fatal.
- Errors to stderr as `chatjimmy: error: <message>`; results to stdout; `--stats` JSON to stderr. `-c 0` is treated as 1.
- Core change allowed: derive `Serialize` on `Stats` only (needed to print stats; no behavior change).
- Process tests spawn the binary via `env!("CARGO_BIN_EXE_chatjimmy")` and must use `#[tokio::test(flavor = "multi_thread")]` so the mock server keeps running while the child is awaited.
- CI tests never touch the real service; the live test is `#[ignore]`.

## Review Focus

1. Empty batch input (no lines): no output, exit 0 (Task 3).
2. Reader closes stdout early (`| head -1`): `run_batch` stops without panic or error exit (Task 2, writer that returns `BrokenPipe`).
3. CRLF input files: trailing `\r` must not end up in prompts or break JSON detection (Task 1).
4. A JSON request whose `prompt` contains an escaped newline sends the newline intact (Task 1).
5. Input file that is not valid UTF-8: fatal exit 1 with a stderr message, not a panic (Task 4).

## File Structure

```
Cargo.toml                       add "crates/cli" to members
crates/core/src/types.rs         derive Serialize on Stats
crates/cli/Cargo.toml
crates/cli/src/main.rs           clap definitions, client construction, dispatch, exit-code mapping
crates/cli/src/ask.rs            prompt resolution, plain and streaming print
crates/cli/src/batch.rs          Defaults, parse_line, Record, run_batch
crates/cli/tests/cli.rs          process tests (wiremock)
crates/cli/tests/live.rs         #[ignore] live test
```

### Task 1: Crate skeleton and `parse_line`

**Files:** Create `crates/cli/Cargo.toml`, `src/main.rs` (minimal `fn main() {}` plus `mod batch;`), `src/batch.rs`; Modify root `Cargo.toml`; Test: `batch.rs` `#[cfg(test)]`.

**Interfaces:**
- Produces:
  - `pub struct Defaults { pub model: String, pub system_prompt: String, pub top_k: u32 }`.
  - `pub fn parse_line(line: &str, d: &Defaults) -> Result<(chatjimmy::ChatRequest, Option<String>), String>`: `Err` carries the message for the error record. Trims a trailing `\r`. The returned `Option<String>` is the request `id`.

Detection: after trimming, if the line parses as a JSON object (`serde_json::Value::Object`) that has a `prompt` key, it is a request, otherwise the whole line is a plain prompt. A `prompt` that is not a string, or an `id`/`system_prompt`/`model` that is present but not a string, or a `top_k` that is not a non-negative integer, gives `Err("...")`. `id` may be a string only.

- [ ] **Step 1: Write failing tests:** `plain_line_uses_defaults`; `json_line_all_fields` (`{"prompt":"p","id":"x","system_prompt":"s","model":"m","top_k":3}` gives request with those values and `Some("x")`); `object_without_prompt_is_plain` (`{"a":1}` is the prompt text); `non_json_is_plain` (`{oops` is the prompt text); `non_string_prompt_is_error` (`{"prompt":5}`); `bad_top_k_is_error` (`{"prompt":"p","top_k":-1}`); `overrides_beat_defaults`; `crlf_is_trimmed` (`"hi\r"` gives prompt `hi`, and `"{\"prompt\":\"p\"}\r"` is detected as JSON); `escaped_newline_preserved` (`{"prompt":"a\nb"}` gives prompt with a real newline). Assert via `req.messages[0].content`, `req.model`, `req.system_prompt`, `req.top_k`.
- [ ] **Step 2:** `cargo test -p chatjimmy-cli` — expect FAIL (not defined).
- [ ] **Step 3: Implement** `parse_line` per the detection rule above.
- [ ] **Step 4:** `cargo test -p chatjimmy-cli` — expect 9 passed.
- [ ] **Step 5: Commit** `feat(cli): crate skeleton and line parser`.

### Task 2: `Record` and `run_batch`

**Files:** Modify `crates/core/src/types.rs` (derive `Serialize` on `Stats`), `crates/cli/src/batch.rs`; Test: same files.

**Interfaces:**
- Consumes: Task 1 `Defaults`, `parse_line`; core `Client::chat_many_stream`, `ChatResponse`.
- Produces:
  - `pub enum Record { Ok { index: usize, id: Option<String>, text: String, stats: Option<Stats> }, Err { index: usize, id: Option<String>, error: String } }` with `fn to_line(&self) -> String` (compact JSON, no trailing newline, `id` omitted when `None`, keys exactly as in the Global Constraints, `stats` serialized as an object or `null`).
  - `pub async fn run_batch(client: &Client, lines: impl IntoIterator<Item = String>, d: &Defaults, concurrency: usize, out: &mut impl std::io::Write) -> std::io::Result<bool>`: returns `Ok(all_ok)`; propagates write errors as `Err`. Blank (whitespace-only) lines are skipped and not counted in `index`. Unparseable lines are written as error records first, before any network result; valid requests go to `chat_many_stream`, a side table maps submission index back to original index and `id`. Each record is written then flushed immediately.

(The spec's `run_batch -> ExitCode` is realized as `io::Result<bool>` here and mapped to `ExitCode` in Task 3/4: same behavior, testable without comparing `ExitCode`.)

- [ ] **Step 1: Write failing tests:** `record_ok_shape` (exact JSON string for an Ok record with id and stats-less `null`); `record_err_shape_omits_missing_id`; `stats_serializes` (core: `serde_json::to_value(Stats::default())` has key `decode_tokens`); `blank_lines_skipped_and_indices_counted` (lines `["", "{\"prompt\":5}", "   ", "{\"prompt\":6}"]` produce two error records with `index` 0 and 1, returns `Ok(false)`, no network used: `Client::new()`); `empty_input_writes_nothing` (returns `Ok(true)`, empty output); `broken_pipe_stops_quietly` (writer returning `ErrorKind::BrokenPipe` on first write: `run_batch` returns `Err` with that kind and does not panic); `ids_and_indices_map_back` (wiremock server; lines `["a", "{\"prompt\":\"b\",\"id\":\"B\"}"]`: each output record's `index`/`id` matches its input regardless of arrival order).
- [ ] **Step 2:** `cargo test -p chatjimmy-cli -p chatjimmy` — expect FAIL.
- [ ] **Step 3: Implement** the derive, `Record`, and `run_batch` (use `futures::StreamExt` over `chat_many_stream`).
- [ ] **Step 4:** same command — expect all pass.
- [ ] **Step 5: Commit** `feat(cli): batch records and runner`.

### Task 3: `clap` wiring, `ask`, and `batch` command

**Files:** Create `src/ask.rs`; Modify `src/main.rs`; Test `tests/cli.rs`.

**Interfaces:**
- Consumes: Tasks 1-2; core `Client`, `chat`, `chat_stream`, `Chunk`.
- Produces: the `Cli`/`Command` clap structs per Global Constraints; `ask::run(client, prompt: String, model, system, top_k, stream: bool, stats: bool) -> chatjimmy::Result<()>`; `main` builds the client with `Client::builder().base_url(--base-url)`, maps results to exit codes (`ask` 0/1, `batch` 0/2, fatal 1), prints errors as `chatjimmy: error: <message>` to stderr. `batch` reads lines from `FILE` or stdin and calls `run_batch` with a locked stdout.

- [ ] **Step 1: Write failing process tests** (mock `POST /api/chat` returns `hi there<|stats|>{"decode_tokens":2}<|/stats|>`): `ask_prints_reply` (stdout `hi there\n`, exit 0); `ask_stream_prints_same_text`; `ask_reads_stdin_when_dash`; `ask_empty_prompt_is_error` (stdin empty: exit 1, stderr starts `chatjimmy: error:`); `ask_stats_goes_to_stderr_as_json` (stderr contains `"decode_tokens":2`, stdout unchanged); `batch_mixed_records_and_exit_2` (stdin: `ok one`, `{"prompt":"two","id":"B"}`, `{"prompt":5}`; mock fails the prompt `two` with 400: three records parsed from stdout with correct `index`/`id`/`ok`, exit code 2); `batch_all_ok_exit_0`; `batch_empty_input_exit_0_no_output` (Review Focus 1); `batch_concurrency_zero_completes` (`-c 0`).
- [ ] **Step 2:** `cargo test -p chatjimmy-cli --test cli` — expect FAIL.
- [ ] **Step 3: Implement** `ask.rs` and `main.rs`. `ask` non-stream uses `chat`; `--stream` prints each `Chunk::Text` as it arrives and captures `Chunk::Done` for `--stats`; both end with `\n`.
- [ ] **Step 4:** `cargo test -p chatjimmy-cli` — all pass.
- [ ] **Step 5: Commit** `feat(cli): ask and batch commands`.

### Task 4: Fatal paths, pipe handling, live test, README

**Files:** Modify `src/main.rs`; Create `tests/live.rs`; Modify `README.md`; Test `tests/cli.rs`.

**Interfaces:** Consumes Task 3 `main`. Produces: documented behavior; no new public API.

- [ ] **Step 1: Write failing tests:** `batch_missing_file_exit_1` (stderr contains the path, starts `chatjimmy: error:`); `batch_invalid_utf8_file_exit_1` (Review Focus 5; file bytes `ff fe`: exit 1, no panic text on stderr); `batch_reads_file_argument`; and `tests/live.rs` `#[ignore] live_ask_cli` running `chatjimmy ask "Say hi"` against the real service, asserting exit 0 and non-empty stdout.
- [ ] **Step 2:** `cargo test -p chatjimmy-cli --test cli` — expect FAIL for the new tests.
- [ ] **Step 3: Implement:** read the input file as bytes and convert with `String::from_utf8`, mapping failure to a fatal error; in `main`, map `run_batch`'s `Err(BrokenPipe)` to exit 0 silently and any other `Err` to exit 1 with a message.
- [ ] **Step 4:** `cargo test` (whole workspace, all pass) and `cargo test -p chatjimmy-cli -- --ignored` (report honestly if the service is down). Run `cargo clippy --all-targets` (clean).
- [ ] **Step 5:** Add a "CLI" section to `README.md`: install (`cargo install --path crates/cli`), `ask` and `--stream` examples, the batch input/output formats, exit codes, and the plain-prompt-looks-like-JSON limitation. Commit `feat(cli): fatal paths, live test, docs`.
