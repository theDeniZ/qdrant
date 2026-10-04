"""Boot-time setup of the server's own working directories on ``/data``.

The volume holds only what the server needs to run: ``keys.db`` (API keys,
``app/keystore.py``), the fastembed model cache, and the import pipeline's
working directories below. Corpus data and every piece of metadata about it
live in Qdrant only — nothing about the corpus is kept here.
"""

from __future__ import annotations

import logging
import os
from pathlib import Path

log = logging.getLogger("seed")

# Directories the import pipeline writes into, all on the persistent volume.
DATA_DIRS = ("packs", "jobs", "uploads")


def ensure_data_dirs(root: Path | None = None) -> list[Path]:
    """Create the volume's working directories at **runtime**.

    The Dockerfile also creates these, but that only ever helps a *fresh* named
    volume: Docker copies an image directory's contents into a named volume only
    when that volume is first created. The deployed ``qdrant-mcp-data`` volume
    already exists, so a rebuilt image's new directories would never appear in
    it and the first import would fail on a missing path. Creating them here, on
    every boot, is what actually guarantees they exist.
    """
    root = root or Path(os.environ.get("DATA_ROOT", "/data"))
    made = []
    for name in DATA_DIRS:
        path = root / name
        try:
            if not path.is_dir():
                path.mkdir(parents=True, exist_ok=True)
                made.append(path)
                log.info("Created %s", path)
        except OSError as e:
            log.error("Cannot create %s: %s", path, e)
    return made


# Files earlier versions kept on the volume to describe the corpus. That
# metadata now lives in Qdrant only (book metadata on every point, the
# contract fingerprint in the collection's own metadata), so a leftover copy
# is removed at boot rather than left to drift.
LEGACY_FILES = ("sop_books.json", "contract_fingerprints.json")


def remove_legacy_files(root: Path | None = None) -> list[Path]:
    root = root or Path(os.environ.get("DATA_ROOT", "/data"))
    removed = []
    for name in LEGACY_FILES:
        path = root / name
        try:
            if path.is_file():
                path.unlink()
                removed.append(path)
                log.info("Removed obsolete %s (corpus metadata lives in Qdrant)", path)
        except OSError as e:
            log.error("Cannot remove %s: %s", path, e)
    return removed
