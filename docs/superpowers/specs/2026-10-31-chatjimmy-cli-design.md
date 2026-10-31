# chatjimmy-cli: sub-project 2

## Context

A command-line binary on top of `chatjimmy-core` (sub-project 1, merged). It adds no new core features. Sub-project 3 (PyO3 bindings) is separate.

Layout: `crates/cli/`, package `chatjimmy-cli`, binary name `chatjimmy`. Added to the workspace `members`. Dependencies: `chatjimmy`, `clap` (derive), `tokio`, `serde`, `serde_json`, `futures`. Dev: `wiremock`, `tokio`.

## Goals

- Non-streaming is the default for single prompts; `--stream` is opt-in.
- A batch mode that runs many requests concurrently and writes JSONL results.
- Result data on stdout, diagnostics on stderr, so pipes work.

## Command surface

```
chatjimmy ask   [PROMPT] [--stream] [--stats]
chatjimmy batch [FILE]   [-c/--concurrency 16]
shared options: --model, --system, --top-k, --base-url
```

### `ask`

- Prompt comes from the argument, or from stdin when the argument is `-` or omitted.
- Default: print the full reply once. `--stream`: print text as it arrives. Both end with a trailing newline, text to stdout.
- `--stats`: print the stats as one JSON object to stderr.
- Exit 0 on success, 1 on any error (message on stderr).

### `batch`

- Input: `FILE`, or stdin if omitted. Blank lines are skipped.
- Each line is a plain prompt or a JSON request, detected per line: a line that parses as a JSON object with a string `prompt` field is a request; anything else is a plain prompt. A plain prompt that is itself a JSON object with a string `prompt` field is therefore treated as a request (documented limitation).
- A JSON request may carry `prompt` (required), `id`, `system_prompt`, `model`, `top_k`. Per-line fields override the shared options. A JSON object whose `prompt` is present but not a string becomes an error record for that line; the batch continues. A JSON object with no `prompt` field is a plain prompt.
- Output: one JSON line per request on stdout, written and flushed as each finishes, in completion order:
  - success: `{"index":3,"id":"x","ok":true,"text":"...","stats":{...}}`
  - failure: `{"index":3,"id":"x","ok":false,"error":"..."}`
  - `index` is the 0-based position among non-blank input lines. `id` appears only if the request supplied one.
- Concurrency defaults to 16 (`-c`); `-c 0` is treated as 1 (as in the library).
- Exit codes: 0 if every request succeeded, 2 if any failed (including unparseable lines), 1 for fatal errors (unreadable file, client cannot be built).

## Structure

- `main.rs`: `clap` definitions, builds the `Client` from `--base-url`, dispatches.
- `ask.rs`: prompt resolution (argument or stdin) and the plain or streaming print.
- `batch.rs`:
  - `parse_line(line, &Defaults) -> Result<(ChatRequest, Option<String>), String>`, pure.
  - `Record`, serializing to the two shapes above.
  - `run_batch(client, lines, out: impl Write, concurrency) -> ExitCode`, generic over the writer so tests capture output in a `Vec<u8>`.

## Batch data flow

- All input lines are read first, because `chat_many_stream` takes a `Vec`. The whole input sits in memory (known ceiling; chunking is out of scope).
- Lines that fail to parse become error records immediately and are never submitted. Only valid requests are submitted; a side table maps the library's submission index back to the original line index and `id`.
- Records are written and flushed as results arrive.
- A write failure (for example the reader closed the pipe with `| head`) stops the run quietly, with no panic message.

## Errors

- Messages go to stderr as `chatjimmy: error: <message>`, using the library error's `Display`.
- In batch mode a per-request failure is data (`"ok": false`), not an error. Only fatal problems stop the run.

## Testing

- Unit tests (no network): plain line; JSON line with all fields; object without `prompt` is a plain prompt; non-JSON line is a plain prompt; non-string `prompt` is an error; per-line `model`/`top_k`/`system_prompt` override shared options; `Record` serialization (`id` omitted when absent, both shapes exact); `run_batch` into a `Vec<u8>` with blank lines skipped and indices counted correctly.
- Process tests with `wiremock`, running the built binary via `CARGO_BIN_EXE_chatjimmy` with `--base-url` pointing at the mock: `ask` prints the reply; `ask --stream` prints the same text; `ask` reads stdin; a batch with good prompts, one failing request and one malformed line gives the right records and exit code 2; `id` is echoed; `-c 0` completes; a missing input file gives exit 1 with a stderr message.
- One `#[ignore]` live test running `chatjimmy ask "Say hi"` against the real service.

## Out of scope

`models` and `health` subcommands, config files, color, input chunking for huge files, shell completions.
