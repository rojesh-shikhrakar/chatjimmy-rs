# chatjimmy-py Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** An async-first Python package `chatjimmy` (PyO3 + maturin) over `chatjimmy-core`, mirroring its API: `chat`, opt-in streaming, concurrent batch in input or completion order.

**Architecture:** A `cdylib` crate exposes `chatjimmy._chatjimmy`; every awaitable comes from `pyo3_async_runtimes::tokio::future_into_py` on one shared tokio runtime. Streams are Python async-iterator objects wrapping the core streams behind an `Arc<tokio::sync::Mutex<..>>`. Core errors map one-to-one onto a Python exception hierarchy.

**Tech Stack:** Rust 2021, pyo3 (abi3-py310), pyo3-async-runtimes (tokio), maturin, uv; Python tests: pytest, pytest-asyncio, mypy.

**Spec:** `docs/superpowers/specs/2026-10-31-chatjimmy-py-design.md`

## Global Constraints

- Package `chatjimmy-py` at `crates/py/`; add `"crates/py"` to workspace `members`. Extension module `chatjimmy._chatjimmy`, abi3-py310; distribution and import name `chatjimmy`.
- `pyo3` and `pyo3-async-runtimes` must be a matching pair (same pyo3 minor); take the newest pair `cargo add` resolves. If that pair does not support the local Python (3.14), create the venv with `uv python install 3.13` and record the ruling.
- API exactly as in the spec: `Client(base_url=None, connect_timeout=5.0, read_timeout=30.0, timeout=None, max_retries=3)`; `ask`, `chat`, `chat_stream`, `chat_many(reqs, concurrency)`, `chat_many_stream(reqs, concurrency)`, `health`, `models`; `ChatRequest(messages, model="llama3.1-8B", system_prompt="", top_k=8, attachment=None)`; `Message.user/assistant/system`; `Attachment(name, size, content)`.
- Frozen result types (`Message`, `Attachment`, `Stats`, `ChatResponse`, `Model`, `HealthStatus`, `ChatRequest`): attribute assignment raises `AttributeError`.
- `chat_stream` yields `str`; `stream.stats` is `None` until the stream ends, then `Stats | None`. `chat_stream` and `chat_many_stream` are plain methods returning async iterators that connect on first `__anext__`.
- Batch items are `ChatResponse | ChatJimmyError` values; `concurrency` is required, `0` means 1.
- Exceptions: `ChatJimmyError(Exception)` base; `HttpError`, `StatusError(.code:int, .body:str)`, `EmptyResponseError`, `BadStatsError`, `DecodeError`.
- No Python objects held across `.await`; no panics may cross into Python (invalid arguments raise `ValueError`/`TypeError`).
- Tests run via `uv run pytest` from `crates/py`; the extension is built with `uv run maturin develop`. `pytest-asyncio` in `auto` mode. Live test marked `live`, skipped by default.

## Review Focus

1. One `Client` used across two separate `asyncio.run(...)` calls (event loop changes) still works (Task 2).
2. Breaking out of `async for` over `chat_stream` early, then reusing the client: no hang, server sees the close (Task 3).
3. A non-`ChatRequest` item in the list passed to `chat_many` raises `TypeError`, not a panic (Task 4).
4. `Client(read_timeout=-1.0)` / `float("nan")` raise `ValueError`, not a Rust panic (Task 1).
5. A prompt containing a lone surrogate (`"\ud800"`) raises a `UnicodeEncodeError`/`ValueError`, not a panic (Task 2).

## File Structure

```
Cargo.toml                         add "crates/py"; .gitignore adds .venv/, __pycache__/, *.so
crates/py/Cargo.toml               cdylib "_chatjimmy"
crates/py/pyproject.toml           maturin, dev group (pytest, pytest-asyncio, mypy), pytest config
crates/py/python/chatjimmy/__init__.py   re-exports from ._chatjimmy
crates/py/python/chatjimmy/__init__.pyi  hand-written stubs
crates/py/python/chatjimmy/py.typed
crates/py/src/lib.rs               module init
crates/py/src/types.rs             frozen classes + conversions to/from core types
crates/py/src/errors.rs            exception classes + From<chatjimmy::Error>
crates/py/src/client.rs            Client, stream iterator classes
crates/py/tests/conftest.py        asyncio mock HTTP server
crates/py/tests/test_*.py
crates/py/tests/typing_example.py  checked with mypy --strict
```

### Task 1: Scaffold, types, exceptions, client construction

**Files:** Create everything under `crates/py/` listed above except `client.rs` methods beyond construction; Modify root `Cargo.toml`, `.gitignore`; Test `tests/test_types.py`.

**Interfaces:**
- Produces (Python): `Message.user(content)/assistant/system` with `.role`, `.content`; `Attachment(name, size, content)`; `Stats` (same fields as core `Stats`, read-only); `ChatRequest(messages: list[Message], model=..., system_prompt=..., top_k=..., attachment=None)` with read-only attributes of the same names; `ChatResponse(.text, .stats)`; `Model`, `HealthStatus` (`.healthy` property); the exception hierarchy; `Client(...)` constructor validating arguments. Rust: `fn to_py_err(e: chatjimmy::Error) -> PyErr`, and `PyChatRequest::to_core(&self) -> chatjimmy::ChatRequest`.
- `Client(...)` raises `ValueError` for a negative or NaN `connect_timeout`/`read_timeout`/`timeout`, and for a negative `max_retries`.

- [ ] **Step 1: Write failing tests (`tests/test_types.py`, plain pytest):** `test_message_constructors` (`Message.user("x")` has role `"user"`, content `"x"`; same for assistant and system); `test_request_defaults` (`ChatRequest([Message.user("hi")])` has `model == "llama3.1-8B"`, `system_prompt == ""`, `top_k == 8`, `attachment is None`); `test_frozen` (assigning `.content` on `Message` and `.model` on `ChatRequest` raises `AttributeError`); `test_request_rejects_non_message` (`ChatRequest(["hi"])` raises `TypeError`); `test_exception_hierarchy` (each subclass is a `ChatJimmyError` and an `Exception`; `StatusError` has `code` and `body`); `test_client_rejects_bad_numbers` (`Client(read_timeout=-1.0)`, `Client(connect_timeout=float("nan"))`, `Client(max_retries=-1)` each raise; Review Focus 4); `test_client_defaults_construct` (`Client()` constructs).
- [ ] **Step 2:** Create the scaffold (empty module), build with `uv run maturin develop`, run `uv run pytest tests/test_types.py` — expect FAIL (names missing). If the build fails on Python version, apply the Global Constraints fallback and ledger it.
- [ ] **Step 3: Implement** `types.rs`, `errors.rs`, the `Client` constructor in `client.rs`, `lib.rs` module init, and `__init__.py` (`from ._chatjimmy import *` with an explicit `__all__`). Use `#[pyclass(frozen)]` with `#[pyo3(get)]`. Validate floats with `f64::is_finite() && >= 0.0` before `Duration::from_secs_f64`.
- [ ] **Step 4:** `uv run maturin develop && uv run pytest tests/test_types.py` — expect 7 passed.
- [ ] **Step 5: Commit** `feat(py): scaffold, types, exceptions, client construction`.

### Task 2: Mock server, `ask`, `chat`, `health`, `models`

**Files:** Create `tests/conftest.py`, `tests/test_chat.py`; Modify `src/client.rs`.

**Interfaces:**
- Consumes: Task 1 types, `to_py_err`, `PyChatRequest::to_core`.
- Produces: `async Client.ask(prompt: str) -> str`; `async Client.chat(request: ChatRequest) -> ChatResponse`; `async Client.health() -> HealthStatus`; `async Client.models() -> list[Model]`; the shared `Arc<chatjimmy::Client>` held by `Client`. `conftest.py` provides fixture `mock` (async): a raw asyncio HTTP/1.1 server with `mock.url`, `mock.set_chat(handler)` where `handler(body: dict) -> Response` (status, body text, optional per-chunk delays/`asyncio.Event` waits, chunked transfer), `mock.requests` (parsed bodies), `mock.peak_inflight`, and `mock.disconnects` (count of connections closed by the client before the response completed).

- [ ] **Step 1: Write failing tests:** `test_ask_returns_text`; `test_chat_returns_text_and_stats` (`resp.text == "hi"`, `resp.stats.decode_tokens == 2`); `test_chat_sends_request_options` (server body has the model, system prompt and top_k set on the `ChatRequest`); `test_health_and_models`; `test_status_error` (400 gives `StatusError` with `.code == 400`, `.body`); `test_empty_200_is_empty_response_error`; `test_bad_stats_error`; `test_connection_refused_is_http_error` (closed port); `test_client_reused_across_event_loops` (create one `Client`, run `asyncio.run(client.ask(..))` twice against a mock started per loop; Review Focus 1); `test_lone_surrogate_prompt_raises` (`"\ud800"` raises `UnicodeEncodeError` or `ValueError`; Review Focus 5).
- [ ] **Step 2:** `uv run pytest tests/test_chat.py` — expect FAIL.
- [ ] **Step 3: Implement** with `pyo3_async_runtimes::tokio::future_into_py`; extract and convert all Python arguments before entering the future; map `chatjimmy::Error` with `to_py_err` (`StatusError` gets `code`/`body` attributes set on the instance).
- [ ] **Step 4:** `uv run maturin develop && uv run pytest` — all pass.
- [ ] **Step 5: Commit** `feat(py): async ask, chat, health, models`.

### Task 3: Streaming

**Files:** Modify `src/client.rs`; Test `tests/test_stream.py`.

**Interfaces:**
- Consumes: Task 2 client and mock. Produces: `Client.chat_stream(request: ChatRequest) -> ChatStream` (plain method); `ChatStream` implements `__aiter__` (returns itself) and `__anext__` (awaitable yielding `str` or raising `StopAsyncIteration`), plus property `stats: Stats | None`. Internally `Arc<tokio::sync::Mutex<State>>` over the core stream and the captured `Chunk::Done` stats.

- [ ] **Step 1: Write failing tests:** `test_stream_yields_text_then_stats` (concatenated chunks equal `chat().text`; `stream.stats.decode_tokens == 7` after the loop); `test_stats_none_mid_stream`; `test_stream_text_arrives_before_rest_of_body` (mock holds the tail behind an `asyncio.Event`; first chunk is received before the event is set); `test_stream_without_stats_block` (`stats is None` after the loop); `test_stream_connection_error_surfaces_on_first_anext` (`chat_stream(...)` itself does not raise; `async for` does, with `HttpError`); `test_mid_stream_disconnect_raises` (server closes after a partial chunked body); `test_early_break_cancels_request_and_client_still_works` (break after the first chunk, `mock.disconnects` becomes 1 within 2 s, a following `ask` succeeds; Review Focus 2).
- [ ] **Step 2:** `uv run pytest tests/test_stream.py` — expect FAIL.
- [ ] **Step 3: Implement** `ChatStream` as a `#[pyclass]`; `__anext__` returns a `future_into_py` future that locks the mutex, creates the core stream on first use, and polls `next()`.
- [ ] **Step 4:** `uv run maturin develop && uv run pytest` — all pass.
- [ ] **Step 5: Commit** `feat(py): chat_stream async iterator`.

### Task 4: Batch

**Files:** Modify `src/client.rs`; Test `tests/test_batch.py`.

**Interfaces:**
- Consumes: Tasks 2-3. Produces: `async Client.chat_many(requests: list[ChatRequest], concurrency: int) -> list[ChatResponse | ChatJimmyError]`; `Client.chat_many_stream(requests, concurrency) -> BatchStream` (async iterator of `tuple[int, ChatResponse | ChatJimmyError]`). Both validate every item is a `ChatRequest` up front (`TypeError` naming the index) and clamp `concurrency` to at least 1 (negative values also give 1).

- [ ] **Step 1: Write failing tests:** `test_chat_many_preserves_input_order` (first request slow); `test_chat_many_stream_completion_order`; `test_failed_item_is_exception_value` (middle request 400: `isinstance(r, StatusError)`, `r.code == 400`, neighbors are `ChatResponse`); `test_concurrency_cap` (20 requests at 4: `mock.peak_inflight` in `2..=4`); `test_concurrency_zero_completes`; `test_empty_batch` (`[]`, and the stream ends immediately); `test_non_request_item_is_type_error` (`chat_many([req, "x"], 2)` raises `TypeError` mentioning index 1; Review Focus 3); `test_gather_50_concurrent_ask` (`asyncio.gather` of 50 `ask` calls all return).
- [ ] **Step 2:** `uv run pytest tests/test_batch.py` — expect FAIL.
- [ ] **Step 3: Implement** on the core `chat_many_stream`; convert each result to a Python object under the GIL on arrival.
- [ ] **Step 4:** `uv run maturin develop && uv run pytest` — all pass.
- [ ] **Step 5: Commit** `feat(py): chat_many and chat_many_stream`.

### Task 5: Cancellation, stubs, typing check, live test, README

**Files:** Create `python/chatjimmy/__init__.pyi`, `python/chatjimmy/py.typed`, `tests/typing_example.py`, `tests/test_cancel.py`, `tests/test_live.py`; Modify `README.md`.

**Interfaces:** Consumes the whole Python API. Produces stubs covering every public name with async signatures exactly as in the Global Constraints.

- [ ] **Step 1: Write failing tests:** `test_wait_for_timeout_cancels_request` (`asyncio.wait_for(client.chat(slow_req), 0.1)` raises `TimeoutError` and `mock.disconnects == 1` within 2 s); `test_task_cancel_during_batch` (cancel a `chat_many` task: `CancelledError` propagates, and in-flight mock requests see disconnects); `test_typing_example_passes_mypy_strict` (runs `mypy --strict tests/typing_example.py` in a subprocess, asserts exit 0); `test_live_ask` marked `@pytest.mark.live` (real `Client().ask("Say hi")` is non-empty; skipped unless `-m live`). `typing_example.py` uses every public API with annotations (including `async for` over both streams and `isinstance` narrowing of batch results).
- [ ] **Step 2:** `uv run pytest tests/test_cancel.py` — expect FAIL for the mypy test (no stubs); the cancellation tests may already pass (note this in the ledger if so).
- [ ] **Step 3: Write** `__init__.pyi` and `py.typed`; make sure `pyproject.toml` includes them in the wheel.
- [ ] **Step 4:** `uv run pytest` (all pass, live skipped), `uv run pytest -m live` (report honestly if the service is down), `cargo clippy --all-targets` for the workspace (clean), `uv run maturin build --release` (a wheel is produced).
- [ ] **Step 5:** Add a "Python" section to `README.md`: install (`maturin develop` / the built wheel), `ask`, `chat`, streaming, `chat_many`, error handling, and the pre-publish PyPI-name note. Commit `feat(py): stubs, cancellation tests, live test, docs`.
