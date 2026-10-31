"""Same workload as benches/overhead.rs through a requests-based client (the old Python approach).

usage: python bench/python_baseline.py http://127.0.0.1:8099
"""
import sys
import time
from concurrent.futures import ThreadPoolExecutor

import requests

base = sys.argv[1].rstrip("/")
body = {
    "messages": [{"role": "user", "content": "hi"}],
    "chatOptions": {"selectedModel": "llama3.1-8B", "systemPrompt": "", "topK": 8},
    "attachment": None,
}
session = requests.Session()


def one(_):
    r = session.post(f"{base}/api/chat", json=body, stream=True, timeout=30)
    r.raise_for_status()
    return b"".join(r.iter_content(chunk_size=256))


for n in (1, 16, 128):
    for conc in (1, 8, 32):
        best = None
        for _ in range(5):
            t = time.perf_counter()
            with ThreadPoolExecutor(conc) as pool:
                assert all(pool.map(one, range(n)))
            dt = time.perf_counter() - t
            best = dt if best is None else min(best, dt)
        print(f"n={n:<4} conc={conc:<3} {best * 1000:8.1f} ms  {n / best:8.0f} req/s")
