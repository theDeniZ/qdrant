"""Seed-on-boot for the SoP title table.

The `sop_books.json` file is built into the Docker image at `app/data/sop_books.json`,
but the authoritative copy must live on the persistent `/data` volume to survive
container rebuilds. This module seeds the volume copy from the packaged version on
first boot if it does not yet exist.
"""

from __future__ import annotations

import json
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


def seed_book_titles(volume_path: Path | None = None, packaged_path: Path | None = None) -> None:
    """Seed `/data/sop_books.json` from the packaged copy if it does not exist.

    This runs once at server startup. The packaged `app/data/sop_books.json` is
    never written to directly; all server-side mutations write to the volume copy.

    Args:
        volume_path: Override the volume path (for testing). Defaults to /data/sop_books.json.
        packaged_path: Override the packaged path (for testing). Defaults to app/data/sop_books.json.
    """
    if volume_path is None:
        volume_path = Path("/data/sop_books.json")
    if packaged_path is None:
        here = Path(__file__).resolve().parent
        packaged_path = here / "data" / "sop_books.json"

    # Volume copy already exists — nothing to do.
    if volume_path.is_file():
        log.info("SoP title table already exists at %s", volume_path)
        return

    if not packaged_path.is_file():
        log.warning("Packaged SoP title table not found at %s", packaged_path)
        return

    # Seed the volume from the packaged copy.
    try:
        content = packaged_path.read_text(encoding="utf-8")
        # Validate that it's valid JSON before writing.
        json.loads(content)
        volume_path.parent.mkdir(parents=True, exist_ok=True)
        volume_path.write_text(content, encoding="utf-8")
        log.info("Seeded SoP title table from %s to %s", packaged_path, volume_path)
    except json.JSONDecodeError as e:
        log.error("Packaged SoP title table contains invalid JSON: %s", e)
    except OSError as e:
        log.error("Failed to seed SoP title table: %s", e)
