"""SQLite-backed API-key store.

Keys are shown once at creation and stored only as a SHA-256 hash (they are
256-bit random tokens, so a fast hash is sufficient). Revoking sets
``revoked_at``; revoked keys stay listed for the audit trail and can be deleted.

OAuth tokens (``app/auth.py``) are derived from a key: each access/refresh
token row points at the ``api_keys`` row that was presented when it was issued,
and is only valid while that key is. Revoking or deleting a key therefore ends
every OAuth session made with it on the next request.
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
        c.execute("""
            CREATE TABLE IF NOT EXISTS oauth_tokens (
                token_hash   TEXT PRIMARY KEY,
                kind         TEXT NOT NULL,          -- 'access' | 'refresh'
                key_id       INTEGER NOT NULL,
                client_id    TEXT NOT NULL,
                created_at   REAL NOT NULL,
                expires_at   REAL NOT NULL
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
        cur = c.execute("DELETE FROM api_keys WHERE id=? AND revoked_at IS NOT NULL", (key_id,))
        if cur.rowcount:
            c.execute("DELETE FROM oauth_tokens WHERE key_id=?", (key_id,))


def list_keys() -> list[dict]:
    with _conn() as c:
        return [dict(r) for r in c.execute("SELECT id, name, prefix, created_at, revoked_at, "
                                           "last_used_at FROM api_keys ORDER BY created_at DESC")]


def _touch(c: sqlite3.Connection, key_id: int) -> None:
    now = time.time()
    if now - _last_touch.get(key_id, 0) > _TOUCH_INTERVAL_S:
        _last_touch[key_id] = now
        c.execute("UPDATE api_keys SET last_used_at=? WHERE id=?", (now, key_id))


def key_id(key: str) -> int | None:
    """Return the id of *key* if it is a valid, unrevoked API key (no OAuth tokens)."""
    if not key:
        return None
    with _conn() as c:
        row = c.execute("SELECT id FROM api_keys WHERE key_hash=? AND revoked_at IS NULL",
                        (_hash(key),)).fetchone()
    return row["id"] if row else None


def verify(key: str) -> str | None:
    """Return the key's name if *key* is a valid API key or an unexpired OAuth
    access token of one, and the key is not revoked; else None."""
    if not key:
        return None
    with _conn() as c:
        if key.startswith(ACCESS_PREFIX):
            row = c.execute(
                "SELECT k.id, k.name FROM oauth_tokens t JOIN api_keys k ON k.id = t.key_id "
                "WHERE t.token_hash=? AND t.kind='access' AND t.expires_at>? "
                "AND k.revoked_at IS NULL", (_hash(key), time.time())).fetchone()
        else:
            row = c.execute("SELECT id, name FROM api_keys WHERE key_hash=? AND revoked_at IS NULL",
                            (_hash(key),)).fetchone()
        if row is None:
            return None
        _touch(c, row["id"])
    return row["name"]


# ── OAuth tokens ─────────────────────────────────────────────────────────────

ACCESS_PREFIX = "qda_"
REFRESH_PREFIX = "qdr_"


def issue_tokens(key_id: int, client_id: str, access_ttl: float,
                 refresh_ttl: float) -> tuple[str, str]:
    """Create an access + refresh token pair bound to *key_id*; return both plaintexts."""
    access = ACCESS_PREFIX + secrets.token_urlsafe(32)
    refresh = REFRESH_PREFIX + secrets.token_urlsafe(32)
    now = time.time()
    with _lock, _conn() as c:
        c.execute("DELETE FROM oauth_tokens WHERE expires_at<=?", (now,))
        c.executemany(
            "INSERT INTO oauth_tokens (token_hash, kind, key_id, client_id, created_at, expires_at) "
            "VALUES (?,?,?,?,?,?)",
            [(_hash(access), "access", key_id, client_id, now, now + access_ttl),
             (_hash(refresh), "refresh", key_id, client_id, now, now + refresh_ttl)])
    return access, refresh


def consume_refresh(refresh: str) -> tuple[int, str] | None:
    """Invalidate *refresh* and return ``(key_id, client_id)`` if it was valid and its
    key is unrevoked. Single use: the caller issues a fresh pair (rotation)."""
    if not refresh.startswith(REFRESH_PREFIX):
        return None
    with _lock, _conn() as c:
        row = c.execute(
            "SELECT t.key_id, t.client_id FROM oauth_tokens t JOIN api_keys k ON k.id = t.key_id "
            "WHERE t.token_hash=? AND t.kind='refresh' AND t.expires_at>? "
            "AND k.revoked_at IS NULL", (_hash(refresh), time.time())).fetchone()
        c.execute("DELETE FROM oauth_tokens WHERE token_hash=?", (_hash(refresh),))
    return (row["key_id"], row["client_id"]) if row else None


def oauth_sessions() -> dict[int, int]:
    """Live refresh tokens per key id (one per connected OAuth client)."""
    with _conn() as c:
        return {r["key_id"]: r["n"] for r in c.execute(
            "SELECT key_id, COUNT(*) AS n FROM oauth_tokens "
            "WHERE kind='refresh' AND expires_at>? GROUP BY key_id", (time.time(),))}
