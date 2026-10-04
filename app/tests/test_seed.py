"""Test the boot-time setup of the volume's working directories."""

from __future__ import annotations

import sys
import tempfile
from pathlib import Path

# Run standalone (`python app/tests/test_seed.py`) as well as `-m app.tests.test_seed`:
# executing the file directly puts app/tests/ on sys.path, not the repo root.
sys.path.insert(0, str(Path(__file__).resolve().parents[2]))

from app.seed import DATA_DIRS, LEGACY_FILES, ensure_data_dirs, remove_legacy_files  # noqa: E402


def test_ensure_data_dirs():
    """The volume working directories are created at runtime, and the call is
    idempotent — the deployed named volume already exists, so the Dockerfile's
    mkdir never reaches it."""
    with tempfile.TemporaryDirectory() as tmpdir:
        root = Path(tmpdir) / "data"
        root.mkdir()

        made = ensure_data_dirs(root=root)
        assert len(made) == len(DATA_DIRS), f"expected {len(DATA_DIRS)} dirs, made {made}"
        for name in DATA_DIRS:
            assert (root / name).is_dir(), f"{name} was not created"

        again = ensure_data_dirs(root=root)
        assert again == [], f"second call should be a no-op, made {again}"
        print("✓ test_ensure_data_dirs passed")


def test_remove_legacy_files():
    """Corpus metadata files from earlier versions are deleted at boot; the
    server's own files (keys.db, working dirs) are left alone."""
    with tempfile.TemporaryDirectory() as tmpdir:
        root = Path(tmpdir)
        for name in LEGACY_FILES:
            (root / name).write_text("{}", encoding="utf-8")
        (root / "keys.db").write_text("keep", encoding="utf-8")
        (root / "jobs").mkdir()

        removed = remove_legacy_files(root=root)
        assert sorted(p.name for p in removed) == sorted(LEGACY_FILES), removed
        for name in LEGACY_FILES:
            assert not (root / name).exists(), f"{name} survived"
        assert (root / "keys.db").is_file() and (root / "jobs").is_dir()
        assert remove_legacy_files(root=root) == [], "second call should be a no-op"
        print("✓ test_remove_legacy_files passed")


if __name__ == "__main__":
    test_remove_legacy_files()
    test_ensure_data_dirs()
    print("\n✓ All tests passed!")
