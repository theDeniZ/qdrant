"""M0 spike fixture: stored vectors + Python reference tokens/vectors.

Reads the live collections (read-only scroll) and writes fixture/*.json that the
Rust spike compares against. Run with the workspace venv:

    FASTEMBED_CACHE_PATH=/workspaces/sdarm/.fastembed_cache \
      /workspaces/sdarm/.venv/bin/python3.11 make_fixture.py
"""
import json, os, sys, time, urllib.request
from pathlib import Path

Q = os.environ.get("QDRANT_URL", "http://10.10.10.10:6333")
VEC = "fast-multilingual-e5-large"
OUT = Path(__file__).parent / "fixture"


def post(path, body):
    req = urllib.request.Request(f"{Q}{path}", data=json.dumps(body).encode(),
                                 headers={"Content-Type": "application/json"}, method="POST")
    with urllib.request.urlopen(req, timeout=60) as r:
        return json.loads(r.read())["result"]


def scroll(coll, flt, limit, vectors=True, offset=None):
    body = {"limit": limit, "with_payload": True, "with_vector": [VEC] if vectors else False}
    if flt:
        body["filter"] = flt
    if offset is not None:
        body["offset"] = offset
    return post(f"/collections/{coll}/points/scroll", body)


def pt(coll, p, group):
    pl = p["payload"]
    text = pl["raw_text"] if coll == "sop" else pl["text"]
    tag = (f"{pl.get('lang')}:{pl.get('book_code')}:{pl.get('para_key')}" if coll == "sop"
           else f"{pl['bible']}:{pl['osis']}")
    if coll == "sop" and pl.get("corpus"):
        tag += f" [{pl['corpus']}]"
    return {"id": str(p["id"]), "collection": coll, "group": group, "tag": tag,
            "text": text, "stored": p["vector"][VEC]}


points = []
# sop: 4 per language (12 langs), 4 pioneers, offset into the id space for variety
for lang in ["en", "de", "es", "pt", "ru", "ko", "ro", "fr", "it", "ja", "uk", "zh"]:
    r = scroll("sop", {"must": [{"key": "lang", "match": {"value": lang}}],
                       "must_not": [{"is_empty": {"key": "raw_text"}}]}, 4,
               offset="7f000000-0000-0000-0000-000000000000")
    points += [pt("sop", p, f"sop-{lang}") for p in r["points"]]
r = scroll("sop", {"must_not": [{"is_empty": {"key": "corpus"}}]}, 4,
           offset="3a000000-0000-0000-0000-000000000000")
points += [pt("sop", p, "sop-pioneer") for p in r["points"]]
# bibles: 3 per translation
for bible in ["synodal", "luther1912", "schlachter", "japkougo", "elberfelder1905", "kjv",
              "nkjv", "korean", "spanish", "ukrogienko", "net"]:
    r = scroll("bibles", {"must": [{"key": "bible", "match": {"value": bible}}]}, 3,
               offset="55000000-0000-0000-0000-000000000000")
    points += [pt("bibles", p, f"bible-{bible}") for p in r["points"]]

# long texts: longest raw_text among a sample of en + de EGW paragraphs (truncation path)
cands = []
for lang in ["en", "de", "ru"]:
    off = None
    for _ in range(4):
        r = scroll("sop", {"must": [{"key": "lang", "match": {"value": lang}}]}, 500,
                   vectors=False, offset=off)
        cands += r["points"]
        off = r.get("next_page_offset")
cands.sort(key=lambda p: -len(p["payload"].get("raw_text") or ""))
long_ids = [str(p["id"]) for p in cands[:4]]
r = post("/collections/sop/points", {"ids": long_ids, "with_payload": True, "with_vector": [VEC]})
points += [pt("sop", p, "sop-long") for p in r]

# bench: 192 consecutive en+de paragraphs (realistic book-like length mix)
for lang, off in [("en", "11000000-0000-0000-0000-000000000000"),
                  ("de", "c1000000-0000-0000-0000-000000000000")]:
    r = scroll("sop", {"must": [{"key": "lang", "match": {"value": lang}}]}, 96, offset=off)
    points += [pt("sop", p, "bench") for p in r["points"]]

print(f"{len(points)} points fetched", file=sys.stderr)

# Python reference: exactly the tokenizer + model sopack 0.1.x uses
from fastembed import TextEmbedding
import fastembed
model = TextEmbedding(model_name="intfloat/multilingual-e5-large")
tok = model.model.tokenizer
texts = ["passage: " + p["text"] for p in points]

# token ids, each text encoded alone (no padding)
for p, t in zip(points, texts):
    p["py_ids"] = tok.encode(t).ids

# extra tokenizer-only probes: scripts not otherwise covered + an extreme length
extra = {
    "de": "passage: Gott, sei mir gnädig nach deiner Güte, und tilge meine Sünden nach deiner großen Barmherzigkeit.",
    "en": "passage: Have mercy upon me, O God, according to thy lovingkindness: according unto the multitude of thy tender mercies blot out my transgressions.",
    "ja": "passage: 神よ、あなたのいつくしみによって、わたしをあわれみ、あなたの豊かなあわれみによって、わたしのもろもろのとがをぬぐい去ってください。",
    "ko": "passage: 하나님이여 주의 인자를 좇아 나를 긍휼히 여기시며 주의 많은 자비를 좇아 내 죄과를 도말하소서",
    "ru": "passage: Помилуй меня, Боже, по великой милости Твоей, и по множеству щедрот Твоих изгладь беззакония мои.",
    "uk": "passage: Помилуй мене, Боже, з великої ласки Своєї, з великого милосердя Свого сотри беззаконня мої!",
    "mixed": "passage: „Ellen G. White“ — «цитата» 1888 SC 12.3; Ps. 51:1–3 … naïve café Ωμέγα  ​ tab\there",
    "long": "passage: " + " ".join(["Und Gott sprach: Es werde Licht! und es ward Licht."] * 120),
}
extra_ids = {k: tok.encode(v).ids for k, v in extra.items()}

t0 = time.time()
vecs = list(model.embed(texts, batch_size=32))
dt = time.time() - t0
print(f"python embed: {len(texts)} texts in {dt:.1f}s", file=sys.stderr)
for p, v in zip(points, vecs):
    p["py_vec"] = [float(x) for x in v]

meta = {"fastembed": fastembed.__version__, "python_embed_seconds": round(dt, 2),
        "python_batch_size": 32, "n": len(points)}
import onnxruntime, tokenizers
meta["onnxruntime"] = onnxruntime.__version__
meta["tokenizers"] = tokenizers.__version__
meta["truncation"] = tok.truncation
meta["padding"] = tok.padding

OUT.mkdir(exist_ok=True)
(OUT / "points.json").write_text(json.dumps(points, ensure_ascii=False))
(OUT / "extra_tokens.json").write_text(json.dumps({"texts": extra, "ids": extra_ids}, ensure_ascii=False))
(OUT / "meta.json").write_text(json.dumps(meta, indent=2, default=str))
print(json.dumps(meta, default=str), file=sys.stderr)
