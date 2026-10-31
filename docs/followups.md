# Deferred follow-ups for chatjimmy-core

Minor findings from the whole-branch review, deferred on purpose. Each is small; take them as one polish pass.

- [ ] Honor `Retry-After` on 429 instead of the fixed backoff schedule.
- [ ] Comment (or reject) text after `<|/stats|>`, which `Parser::finish` drops silently.
- [ ] Add a distinct error for non-JSON 200 responses; they currently surface as `Error::Http`.
- [ ] Complete the README benchmark: N = 1 and 16 rows, plus p50/p99 latency (spec requires them).
- [ ] Trim the library's `tokio` features to `time`; move `rt-multi-thread` and `macros` to dev-dependencies.
- [ ] Decide whether a real total timeout (wrapping the retry loop) is wanted; `.timeout()` is per attempt today.
- [ ] Decide whether a body with no stats block should be an `Error::Truncated` (proxy-truncation case).
- [ ] Revisit the 8-byte streaming holdback only if final-token latency is reported.

## CLI (from the chatjimmy-cli review)

- [ ] `ask`: trim the argument prompt like the stdin prompt (`ask "   "` currently sends spaces).
- [ ] Validate `--base-url` (http/https) up front so a typo is one fatal error, not one error record per line.
- [ ] `ask --stats` prints nothing when the reply has no stats; print `null` or a note.
- [ ] A mid-`--stream` failure leaves the error on the same line as partial text; emit a newline first.
- [ ] Batch exits 0 after a closed pipe even if already-written records had failed (spec says "stop quietly").
- [ ] Warn on unknown JSON keys in batch requests (typo like `sytem_prompt` is silently ignored).
- [ ] README: mention `-c 0` means 1, and that batch exits 0 when the reader closes the pipe.
