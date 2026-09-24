"""Canary probe source material — R11's proof that the pack's embedding space
still matches the live collection's.

Read-only. **Stdlib only** (``urllib``) — this runs on the Mac before
``fastembed`` is imported by ``sopack.pack``, and the server never touches it
at all (the probe it receives is just numbers + ids).

::

    sopack canaries --qdrant http://10.10.10.10:6333 -o canaries.json   # once

    canaries = load("canaries.json")
    # ... or fetch fresh:
    canaries = fetch("http://10.10.10.10:6333", "sop", n=8)
"""

from __future__ import annotations

import json
import urllib.error
import urllib.request
from pathlib import Path

from . import contract

__all__ = ["fetch", "save", "load", "CanaryError"]


class CanaryError(Exception):
    """Could not obtain canaries from the live collection."""


def _profile_for_collection(collection: str) -> contract.Profile:
    for profile in contract.PROFILES.values():
        if profile.collection == collection:
            return profile
    raise CanaryError(
        f"no profile targets collection {collection!r} "
        f"(known collections: {', '.join(sorted(p.collection for p in contract.PROFILES.values()))})")


def _post(url: str, body: dict, timeout: float) -> dict:
    req = urllib.request.Request(
        url, data=json.dumps(body).encode("utf-8"),
        headers={"Content-Type": "application/json"}, method="POST")
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            raw = resp.read()
    except urllib.error.URLError as exc:
        raise CanaryError(f"request to {url} failed: {exc}") from exc
    try:
        data = json.loads(raw)
    except json.JSONDecodeError as exc:
        raise CanaryError(f"{url} did not return JSON: {exc}") from exc
    if "result" not in data:
        raise CanaryError(f"{url} returned no 'result' (status={data.get('status')!r})")
    return data["result"]


def fetch(qdrant_url: str, collection: str, n: int = 8,
          timeout: float = 30.0, max_pages: int = 10) -> list[dict]:
    """Scroll *collection* for up to *n* points already indexed there.

    Returns ``[{"id", "collection", "text"}, ...]`` — *text* is read from the
    profile's ``text_field`` (``raw_text`` for ``sop``, ``text`` for
    ``bible``). A point whose text field is missing or blank is skipped: a
    canary with no text cannot be re-embedded.
    """
    if n <= 0:
        return []
    profile = _profile_for_collection(collection)
    root = f"{qdrant_url.rstrip('/')}/collections/{collection}"
    out: list[dict] = []
    offset = None
    page = 0
    while len(out) < n and page < max_pages:
        page += 1
        body = {"limit": max(n * 2, 16), "with_payload": True, "with_vector": False}
        if offset is not None:
            body["offset"] = offset
        result = _post(f"{root}/points/scroll", body, timeout)
        points = result.get("points") or []
        for p in points:
            payload = p.get("payload") or {}
            text = payload.get(profile.text_field)
            if not isinstance(text, str) or not text.strip():
                continue
            out.append({"id": str(p["id"]), "collection": collection, "text": text})
            if len(out) >= n:
                break
        offset = result.get("next_page_offset")
        if offset is None or not points:
            break
    if not out:
        raise CanaryError(
            f"no usable points with text found in {collection!r} — "
            "cannot build a canary probe from an empty or textless collection")
    return out


def save(canaries: list[dict], path) -> None:
    Path(path).write_text(
        json.dumps(canaries, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def load(path) -> list[dict]:
    data = json.loads(Path(path).read_text(encoding="utf-8"))
    if not isinstance(data, list):
        raise CanaryError(f"{path}: expected a JSON array of canaries")
    return data
