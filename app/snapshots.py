"""Qdrant snapshot create / list / restore / prune, via plain REST calls.

This is the R2/R3 safety net of docs/IMPORT-PIPELINE.md: a restorable backup
before any mutation, and a restore path that is actually exercised. Matches
the plain-``requests`` style of ``app/sop_tools.py`` — stdlib + ``requests``
only, no qdrant_client.

Snapshot lifecycle (real Qdrant REST API, not guessed):

    POST   /collections/{c}/snapshots              create
    GET    /collections/{c}/snapshots               list
    DELETE /collections/{c}/snapshots/{name}         delete
    PUT    /collections/{c}/snapshots/recover        restore (from a location)

Restore uses Qdrant's own self-referencing download URL as the ``location``
(``{base}/collections/{c}/snapshots/{name}``) so no shared filesystem between
this server and the Qdrant node is required.
"""

from __future__ import annotations

import os

import requests

DEFAULT_RETENTION = 5
_TIMEOUT_S = float(os.environ.get("SNAPSHOT_TIMEOUT_S", "120"))
_RESTORE_TIMEOUT_S = float(os.environ.get("SNAPSHOT_RESTORE_TIMEOUT_S", "900"))


class SnapshotError(Exception):
    """A snapshot operation failed. Callers treat this as fatal — e.g. the
    import 'snapshot' stage aborts the whole job before writing anything."""


def _qdrant_url(qdrant_url: str | None = None) -> str:
    return (qdrant_url or os.environ.get("QDRANT_URL", "http://localhost:6333")).rstrip("/")


def create(collection: str, qdrant_url: str | None = None, wait: bool = True) -> dict:
    """Create a snapshot of *collection*. Raises :class:`SnapshotError` on any
    failure — the whole point of the import 'snapshot' stage is that nothing
    is written if this fails."""
    base = _qdrant_url(qdrant_url)
    try:
        r = requests.post(f"{base}/collections/{collection}/snapshots",
                          params={"wait": "true" if wait else "false"}, timeout=_TIMEOUT_S)
    except requests.RequestException as exc:
        raise SnapshotError(f"snapshot create request failed: {exc}") from exc
    if r.status_code >= 300:
        raise SnapshotError(f"snapshot create failed: HTTP {r.status_code} {r.text[:400]}")
    result = (r.json() or {}).get("result")
    if not result or not result.get("name"):
        raise SnapshotError(f"snapshot create returned no name: {r.text[:400]}")
    return result


def list_snapshots(collection: str, qdrant_url: str | None = None) -> list[dict]:
    base = _qdrant_url(qdrant_url)
    r = requests.get(f"{base}/collections/{collection}/snapshots", timeout=_TIMEOUT_S)
    if r.status_code >= 300:
        raise SnapshotError(f"snapshot list failed: HTTP {r.status_code} {r.text[:400]}")
    return (r.json() or {}).get("result") or []


def delete(collection: str, name: str, qdrant_url: str | None = None) -> None:
    base = _qdrant_url(qdrant_url)
    r = requests.delete(f"{base}/collections/{collection}/snapshots/{name}", timeout=_TIMEOUT_S)
    if r.status_code >= 300 and r.status_code != 404:
        raise SnapshotError(f"snapshot delete failed: HTTP {r.status_code} {r.text[:400]}")


def restore(collection: str, name: str, qdrant_url: str | None = None,
            wait: bool = True) -> dict:
    """Recover *collection* from a snapshot previously created of itself (the
    'coarse' rollback of docs/IMPORT-PIPELINE-PLAN.md §7 — replaces the whole
    collection, losing anything written since)."""
    base = _qdrant_url(qdrant_url)
    location = f"{base}/collections/{collection}/snapshots/{name}"
    try:
        r = requests.put(f"{base}/collections/{collection}/snapshots/recover",
                         params={"wait": "true" if wait else "false"},
                         json={"location": location}, timeout=_RESTORE_TIMEOUT_S)
    except requests.RequestException as exc:
        raise SnapshotError(f"snapshot restore request failed: {exc}") from exc
    if r.status_code >= 300:
        raise SnapshotError(f"snapshot restore failed: HTTP {r.status_code} {r.text[:400]}")
    return (r.json() or {}).get("result") or {}


def prune(collection: str, keep: int = DEFAULT_RETENTION,
          qdrant_url: str | None = None) -> list[str]:
    """Delete all but the *keep* most recent snapshots of *collection*.
    Returns the names deleted."""
    snaps = list_snapshots(collection, qdrant_url)
    snaps.sort(key=lambda s: s.get("creation_time") or "", reverse=True)
    doomed = snaps[keep:]
    for s in doomed:
        delete(collection, s["name"], qdrant_url)
    return [s["name"] for s in doomed]
