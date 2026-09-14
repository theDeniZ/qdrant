"""SQLite-backed API-key store.

Keys are shown once at creation and stored only as a SHA-256 hash (they are
256-bit random tokens, so a fast hash is sufficient). Revoking sets
``revoked_at``; revoked keys stay listed for the audit trail and can be deleted.
"""

from __future__ import annotations

import hashlib
import os
import secrets
import sqlite3
import threading
import time

_DB_PATH = os.environ.get("KEYS_DB", "/data/keys.db")
_TOUCH_INTERVAL_S = 60  # throttle last_used_at writes
_lock = threading.Lock()
_last_touch: dict[int, float] = {}


def _conn() -> sqlite3.Connection:
    c = sqlite3.connect(_DB_PATH, timeout=10)
    c.row_factory = sqlite3.Row
    return c


def init() -> None:
    os.makedirs(os.path.dirname(_DB_PATH) or ".", exist_ok=True)
    with _conn() as c:
        c.execute("""
            CREATE TABLE IF NOT EXISTS api_keys (
                id           INTEGER PRIMARY KEY AUTOINCREMENT,
                name         TEXT NOT NULL UNIQUE,
                key_hash     TEXT NOT NULL UNIQUE,
                prefix       TEXT NOT NULL,
                created_at   REAL NOT NULL,
                revoked_at   REAL,
                last_used_at REAL
            )""")


def _hash(key: str) -> str:
    return hashlib.sha256(key.encode()).hexdigest()


def create(name: str) -> str:
    """Create a key named *name*; return the plaintext key (shown once)."""
    name = name.strip()
    if not name:
        raise ValueError("name must not be empty")
    key = "qd_" + secrets.token_urlsafe(32)
    with _lock, _conn() as c:
        c.execute(
            "INSERT INTO api_keys (name, key_hash, prefix, created_at) VALUES (?,?,?,?)",
            (name, _hash(key), key[:9], time.time()),
        )
    return key


def revoke(key_id: int) -> None:
    with _lock, _conn() as c:
        c.execute("UPDATE api_keys SET revoked_at=? WHERE id=? AND revoked_at IS NULL",
                  (time.time(), key_id))


def delete(key_id: int) -> None:
    with _lock, _conn() as c:
        c.execute("DELETE FROM api_keys WHERE id=? AND revoked_at IS NOT NULL", (key_id,))


def list_keys() -> list[dict]:
    with _conn() as c:
        return [dict(r) for r in c.execute("SELECT id, name, prefix, created_at, revoked_at, "
                                           "last_used_at FROM api_keys ORDER BY created_at DESC")]


def verify(key: str) -> str | None:
    """Return the key's name if *key* is valid and not revoked, else None."""
    if not key:
        return None
    with _conn() as c:
        row = c.execute("SELECT id, name FROM api_keys WHERE key_hash=? AND revoked_at IS NULL",
                        (_hash(key),)).fetchone()
        if row is None:
            return None
        now = time.time()
        if now - _last_touch.get(row["id"], 0) > _TOUCH_INTERVAL_S:
            _last_touch[row["id"]] = now
            c.execute("UPDATE api_keys SET last_used_at=? WHERE id=?", (now, row["id"]))
    return row["name"]
