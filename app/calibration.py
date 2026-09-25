"""Calibration fixture export — SOPACK-AUTONOMY.md §3.1's committed
replacement for the live-canary scroll (formerly ``sopack canaries``).

This is an **importer-side, read-only admin command**, not a sopack command
and not a one-off script (docs/IMPORT-PIPELINE-PLAN.md's no-ad-hoc-scripts
rule). ``sopack/`` must never touch a store at all
(``sopack/tests/test_neutrality.py``); building the fixture necessarily
reads the live store, so that work moved here, where it can be a maintained,
tested command instead of a throwaway script.

::

    python -m app.calibration export \\
        --qdrant http://10.10.10.10:6333 \\
        --out qdrant/sopack-rs/contracts/e5-large-v1/calibration.json

Selection (SOPACK-2-FORMAT.md §3) is **deterministic**: every entry comes
from one fixed Qdrant filter, and Qdrant's own scroll order for a fixed
filter against an unchanged collection is stable ascending-by-id — so
rerunning against the same collection state picks the same points in the
same order (verified against the ``sop``/``bibles`` collections: the same
filter always yields the same first ids). No client-side "take the minimum
of a sample" is needed; the filter alone does the deterministic part.

Never writes to Qdrant. Only ``requests.post/.get`` (scroll, facet, retrieve)
against the two read-only routes it needs.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from datetime import datetime, timezone
from pathlib import Path

import requests

_TIMEOUT_S = 30.0

# Fixed selection plan (SOPACK-2-FORMAT.md §3): 16 entries — EGW en + de,
# several other SoP languages, >=1 pioneer point, several Bible translations,
# >=1 text over the model's 512-token window. Counts sum to 16; see
# `build_fixture`'s docstring for exactly which slot is which.
_OTHER_LANGS_WANTED = 4
_PIONEER_WANTED = 2
_BIBLE_WANTED = 5
_EGW_PER_LANG = 2


class CalibrationExportError(Exception):
    """Could not build a calibration fixture from the live store."""


def _post(qdrant_url: str, collection: str, path: str, body: dict) -> dict:
    url = f"{qdrant_url.rstrip('/')}/collections/{collection}/{path}"
    try:
        r = requests.post(url, json=body, timeout=_TIMEOUT_S)
    except requests.RequestException as exc:
        raise CalibrationExportError(f"POST {url}: {exc}") from exc
    if r.status_code >= 300:
        raise CalibrationExportError(f"POST {url}: HTTP {r.status_code} {r.text[:400]}")
    data = r.json() or {}
    if "result" not in data:
        raise CalibrationExportError(f"{url}: no 'result' in response (status={data.get('status')!r})")
    return data["result"]


def _facet_values(qdrant_url: str, collection: str, key: str) -> list[str]:
    result = _post(qdrant_url, collection, "facet", {"key": key, "exact": True, "limit": 200})
    return sorted(str(h["value"]) for h in (result.get("hits") or []))


def _scroll(qdrant_url: str, collection: str, flt: dict, limit: int, *,
           with_vector: bool = True) -> list[dict]:
    result = _post(qdrant_url, collection, "points/scroll",
                   {"limit": limit, "with_payload": True, "with_vector": with_vector,
                    "filter": flt})
    return result.get("points") or []


def _entry_from_point(point: dict, *, profile: str, text_field: str, note: str,
                      vector_name: str) -> dict | None:
    payload = point.get("payload") or {}
    text = payload.get(text_field)
    if not isinstance(text, str) or not text.strip():
        return None
    vector = point.get("vector")
    if isinstance(vector, dict):
        vector = vector.get(vector_name)
    if not vector:
        return None
    return {
        "id": str(point["id"]), "profile": profile, "uid": None,
        "lang": payload.get("lang"), "note": note, "text": text,
        "vector": list(vector),
    }


def _tokenizer(model_dir: str | Path):
    """Load the pinned model's own tokenizer, so token-length selection uses
    the exact vocabulary the embedding contract is built on (not a proxy like
    word/character count)."""
    from tokenizers import Tokenizer
    path = Path(model_dir) / "tokenizer.json"
    if not path.is_file():
        raise CalibrationExportError(f"tokenizer.json not found under {model_dir}")
    return Tokenizer.from_file(str(path))


def build_fixture(qdrant_url: str, *, model_dir: str | Path, vector_name: str,
                  contract_id: str = "e5-large-v1", passage_prefix: str = "passage: ",
                  max_tokens: int = 512) -> dict:
    """Read-only: builds the calibration fixture doc (``sopack.calibration/1``)
    from the live ``sop`` and ``bibles`` collections. Never mutates anything."""
    entries: list[dict] = []
    seen_ids: set[str] = set()

    def _add(point, *, profile, text_field, note):
        e = _entry_from_point(point, profile=profile, text_field=text_field, note=note,
                              vector_name=vector_name)
        if e is not None and e["id"] not in seen_ids:
            entries.append(e)
            seen_ids.add(e["id"])

    # 1. EGW en + de (no `corpus` key — the EGW convention).
    for lang in ("en", "de"):
        flt = {"must": [{"key": "lang", "match": {"value": lang}},
                        {"is_empty": {"key": "corpus"}}]}
        for p in _scroll(qdrant_url, "sop", flt, _EGW_PER_LANG):
            _add(p, profile="sop", text_field="raw_text", note=f"EGW {lang}")

    # 2. Several other SoP languages (also EGW works — translated corpus).
    all_langs = _facet_values(qdrant_url, "sop", "lang")
    other_langs = [lang for lang in all_langs if lang not in ("en", "de")][:_OTHER_LANGS_WANTED]
    for lang in other_langs:
        flt = {"must": [{"key": "lang", "match": {"value": lang}},
                        {"is_empty": {"key": "corpus"}}]}
        for p in _scroll(qdrant_url, "sop", flt, 1):
            _add(p, profile="sop", text_field="raw_text", note=f"EGW {lang}")

    # 3. >=1 pioneer point (has `corpus`).
    flt = {"must": [{"key": "lang", "match": {"value": "en"}}],
          "must_not": [{"is_empty": {"key": "corpus"}}]}
    for p in _scroll(qdrant_url, "sop", flt, _PIONEER_WANTED):
        _add(p, profile="sop", text_field="raw_text", note="pioneer (corpus)")

    # 4. Several Bible translations.
    bibles = _facet_values(qdrant_url, "bibles", "bible")[:_BIBLE_WANTED]
    for bible in bibles:
        flt = {"must": [{"key": "bible", "match": {"value": bible}}]}
        for p in _scroll(qdrant_url, "bibles", flt, 1):
            _add(p, profile="bible", text_field="text", note=f"bible {bible}")

    # 5. >=1 text over the model's token window — scan a wider pioneer sample
    # (pioneer paragraphs run longest) for the first one whose tokenized
    # length with the passage prefix exceeds max_tokens; deterministic
    # because the scan order is Qdrant's own stable scroll order.
    tok = _tokenizer(model_dir)

    def _n_tokens(text: str) -> int:
        return len(tok.encode(passage_prefix + text).ids)

    long_entry = None
    flt = {"must": [{"key": "lang", "match": {"value": "en"}}],
          "must_not": [{"is_empty": {"key": "corpus"}}]}
    for p in _scroll(qdrant_url, "sop", flt, 6000):
        payload = p.get("payload") or {}
        text = payload.get("raw_text")
        if not isinstance(text, str):
            continue
        if _n_tokens(text) > max_tokens:
            e = _entry_from_point(p, profile="sop", text_field="raw_text",
                                  note=f"pioneer, >{max_tokens} tokens", vector_name=vector_name)
            if e is not None and e["id"] not in seen_ids:
                long_entry = e
                break
    if long_entry is None:
        raise CalibrationExportError(
            f"could not find any point over {max_tokens} tokens in the first 500 "
            "pioneer paragraphs scanned — widen the scan or pick a known long book")
    entries.append(long_entry)
    seen_ids.add(long_entry["id"])

    if len(entries) < 8:
        raise CalibrationExportError(
            f"only {len(entries)} usable fixture entries found (need >= 8) — "
            "the live collections may be smaller than expected")

    return {
        "schema": "sopack.calibration/1",
        "contract": contract_id,
        "created_at": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "source": {"store": "qdrant", "note": "vectors copied from the live collections"},
        "entries": entries,
    }


def export(qdrant_url: str, out_path: str | Path, *, model_dir: str | Path,
          vector_name: str, contract_id: str = "e5-large-v1") -> Path:
    fixture = build_fixture(qdrant_url, model_dir=model_dir, vector_name=vector_name,
                            contract_id=contract_id)
    out = Path(out_path)
    out.parent.mkdir(parents=True, exist_ok=True)
    blob = json.dumps(fixture, ensure_ascii=False, indent=2) + "\n"
    out.write_text(blob, encoding="utf-8")
    return out


def sha256_of(path: str | Path) -> str:
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m app.calibration",
                                 description=__doc__)
    sub = ap.add_subparsers(dest="command", required=True)

    p_export = sub.add_parser("export", help="build calibration.json from the live store")
    p_export.add_argument("--qdrant", required=True, help="Qdrant base URL (read-only)")
    p_export.add_argument("--out", required=True, type=Path)
    p_export.add_argument("--model-dir", required=True,
                          help="directory holding tokenizer.json for the pinned model")
    p_export.add_argument("--vector-name", default="fast-multilingual-e5-large")
    p_export.add_argument("--contract-id", default="e5-large-v1")

    args = ap.parse_args(argv)
    if args.command == "export":
        try:
            out = export(args.qdrant, args.out, model_dir=args.model_dir,
                        vector_name=args.vector_name, contract_id=args.contract_id)
        except CalibrationExportError as exc:
            print(f"export failed: {exc}", file=sys.stderr)
            return 1
        sha = sha256_of(out)
        print(f"wrote {out}")
        print(f"sha256: {sha}")
        return 0
    return 2


if __name__ == "__main__":
    sys.exit(main())
