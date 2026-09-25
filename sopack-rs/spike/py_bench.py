"""Python fastembed 0.8.0 on the same bench texts, batch 1 and batch 32 (fair baseline)."""
import json, sys, time
from fastembed import TextEmbedding
pts = [p for p in json.load(open("fixture/points.json")) if p["group"] == "bench"]
texts = ["passage: " + p["text"] for p in pts]
m = TextEmbedding(model_name="intfloat/multilingual-e5-large")
list(m.embed(texts[:1], batch_size=1))
out = {}
for bs in [1, 32]:
    t0 = time.time(); list(m.embed(texts, batch_size=bs)); s = time.time() - t0
    out[bs] = {"seconds": s, "blocks_per_s": len(texts) / s}
    print(f"python batch_size={bs:<3} {s:6.1f}s {len(texts)/s:5.2f} blocks/s", flush=True)
json.dump(out, open("results/python-bench.json", "w"), indent=2)
