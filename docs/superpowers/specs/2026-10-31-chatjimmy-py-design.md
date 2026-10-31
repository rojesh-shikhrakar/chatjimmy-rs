# chatjimmy-py: sub-project 3

## Context

Python bindings over `chatjimmy-core` (sub-project 1, merged), built with PyO3 and maturin. Async-first and not drop-in compatible with the old Python package (`chatjimmy-api`, which stays untouched). The package lives in this workspace as `crates/py`; the Rust workspace is the single source of truth.

Names: distribution `chatjimmy`, import `chatjimmy` (the user's choice). We cannot verify who owns `chatjimmy` on PyPI or whether it would clash with the old package if both are installed; nothing is published here. Pre-publish step: check the name on PyPI.

## Goals

- Python API that mirrors the core: `chat` (full response, default), opt-in streaming, concurrent batch calls in input order or completion order.
- Real asyncio integration: Rust futures become awaitables on the caller's running event loop; cancelling a task cancels the request.
- Typed: ships `.pyi` stubs and `py.typed`.

## Approach

PyO3 with `pyo3-async-runtimes` (tokio feature): one shared multi-thread tokio runtime inside the module, created lazily. Rejected: a hand-rolled bridge via a background thread and `loop.call_soon_threadsafe` (more code to get wrong around cancellation); blocking-only calls with the GIL released (contradicts async-first).

## Layout

```
crates/py/
  Cargo.toml                 package chatjimmy-py, cdylib "_chatjimmy", abi3-py310
  pyproject.toml             maturin; module-name = "chatjimmy._chatjimmy"; dist name "chatjimmy"
  python/chatjimmy/__init__.py, __init__.pyi, py.typed
  src/lib.rs, types.rs, client.rs, errors.rs
  tests/test_*.py            pytest + pytest-asyncio against a local mock server
```

The workspace root `Cargo.toml` gains `"crates/py"` in `members`.

## Python API

```python
from chatjimmy import Client, ChatRequest, Message

client = Client()   # base_url, connect_timeout, read_timeout, timeout, max_retries (all optional)
text = await client.ask("hi")                                       # -> str
resp = await client.chat(ChatRequest([Message.user("hi")],
                         model=..., system_prompt=..., top_k=..., attachment=...))
                                                                    # -> ChatResponse(.text, .stats)
async for chunk in client.chat_stream(req): ...                     # opt-in; str per text chunk
results = await client.chat_many(reqs, concurrency=16)              # list[ChatResponse | ChatJimmyError], input order
async for i, r in client.chat_many_stream(reqs, concurrency=16): ...  # completion order
await client.health(); await client.models()
```

- `ChatRequest` is the single request type everywhere; defaults match the core (model `llama3.1-8B`, `top_k` 8). `Message.user/assistant/system(content)` constructors. `concurrency` is a required argument (as in the core); `0` is treated as 1.
- `chat_stream` and `chat_many_stream` are plain (non-`async`) methods returning an async iterator. The connection opens on the first `__anext__`, so connection errors surface there. End of stream raises `StopAsyncIteration`. Dropping the iterator early drops the stream and cancels the request.
- `chat_stream` yields `str` text chunks. After the loop ends, `stream.stats` holds the `Stats` (or `None` if the server sent none; `None` mid-stream). This stands in for the core's `Chunk::Done`.
- Batch failures are values: each item is a `ChatResponse` or a `ChatJimmyError` instance (the `asyncio.gather(..., return_exceptions=True)` convention). One failure never loses the rest.
- `Message`, `Attachment`, `Stats`, `ChatResponse`, `Model`, `HealthStatus` are frozen classes with read-only attributes.
- Not included: a blocking wrapper, dict-style messages, context-manager lifecycle.

## Errors

`chatjimmy.ChatJimmyError(Exception)` with subclasses `HttpError`, `StatusError` (`.code: int`, `.body: str`), `EmptyResponseError`, `BadStatsError`, `DecodeError`, mapped one-to-one from the core `Error`. Single calls raise them; batch results hold them as values.

## Async bridge

- Awaitables come from `pyo3_async_runtimes::tokio::future_into_py` on one shared runtime, created lazily and never shut down.
- Cancelling the Python task drops the Rust future, aborting the in-flight request.
- No Python objects are held across `.await`; results convert to Python objects on completion, under the GIL.

## Packaging

`maturin` with `abi3-py310` (matches the old package's `requires-python >=3.10`). Development: `uv run maturin develop`; release: `maturin build --release`. The wheel ships `py.typed` and the `.pyi` stub. A multi-platform wheel matrix is out of scope; testing is on this Linux machine only.

## Testing

`pytest` + `pytest-asyncio` with a small asyncio mock HTTP server in `conftest.py` (chunked and delayed responses):

- `ask` and `chat` return text and stats; assigning to a result attribute raises `AttributeError`.
- `chat_stream` yields text before the rest of the body arrives; `stream.stats` is `None` mid-stream and set after the loop.
- `chat_many` keeps input order; `chat_many_stream` is completion order.
- A failed batch item is a `StatusError` with the right `.code`/`.body`; the other items are intact.
- Concurrency cap holds (mock records peak in-flight requests); `concurrency=0` completes; an empty batch gives `[]`.
- Exception hierarchy: empty 200 raises `EmptyResponseError`, and so on for the other subclasses.
- Cancellation: `asyncio.wait_for(client.chat(slow), 0.1)` raises `TimeoutError` and the mock sees the disconnect.
- `asyncio.gather` of 50 concurrent `ask` calls works.
- `mypy --strict` over a small typed example file checks the stubs.
- One live test, skipped by default, run with `pytest -m live`.

## Out of scope

A blocking wrapper, dict-style messages, publishing to PyPI, the multi-platform wheel matrix, a Python-side benchmark.
