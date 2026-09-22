"""Test the seed-on-boot for the SoP title table."""

from __future__ import annotations

import json
import sys
import tempfile
from pathlib import Path

# Run standalone (`python app/tests/test_seed.py`) as well as `-m app.tests.test_seed`:
# executing the file directly puts app/tests/ on sys.path, not the repo root.
sys.path.insert(0, str(Path(__file__).resolve().parents[2]))

from app.seed import DATA_DIRS, ensure_data_dirs, seed_book_titles  # noqa: E402


def test_seed_creates_file_when_missing():
    """Seed creates the volume file from packaged when volume file is missing."""
    with tempfile.TemporaryDirectory() as tmpdir:
        tmp_path = Path(tmpdir)

        # Set up directory structure
        packaged_dir = tmp_path / "app" / "data"
        packaged_dir.mkdir(parents=True)
        packaged_file = packaged_dir / "sop_books.json"

        volume_dir = tmp_path / "volume"
        volume_dir.mkdir(parents=True)
        volume_file = volume_dir / "sop_books.json"

        test_data = {
            "en": {
                "GC": {"titles": ["The Great Controversy"], "year": 1888},
            },
            "de": {
                "DM": {"titles": ["Der große Kampf"], "en_code": "GC"},
            }
        }

        packaged_file.write_text(json.dumps(test_data), encoding="utf-8")

        # Verify volume file doesn't exist
        assert not volume_file.exists(), "Volume file should not exist initially"

        # Call seed with explicit paths
        seed_book_titles(volume_path=volume_file, packaged_path=packaged_file)

        # Verify volume file now exists and has correct content
        assert volume_file.exists(), "Volume file should exist after seed"
        content = json.loads(volume_file.read_text(encoding="utf-8"))
        assert content == test_data, f"Volume file should contain correct data. Got {content}"
        print("✓ test_seed_creates_file_when_missing passed")


def test_seed_does_not_overwrite_existing():
    """Seed does not overwrite an existing volume file."""
    with tempfile.TemporaryDirectory() as tmpdir:
        tmp_path = Path(tmpdir)

        # Create packaged and volume directories
        packaged_dir = tmp_path / "app" / "data"
        packaged_dir.mkdir(parents=True)
        packaged_file = packaged_dir / "sop_books.json"

        volume_dir = tmp_path / "volume"
        volume_dir.mkdir(parents=True)
        volume_file = volume_dir / "sop_books.json"

        packaged_data = {"en": {"GC": {"titles": ["The Great Controversy"]}}}
        volume_data = {"en": {"SC": {"titles": ["Steps to Christ"]}}}

        packaged_file.write_text(json.dumps(packaged_data), encoding="utf-8")
        volume_file.write_text(json.dumps(volume_data), encoding="utf-8")

        # Call seed with explicit paths
        seed_book_titles(volume_path=volume_file, packaged_path=packaged_file)

        # Verify volume file was NOT changed
        content = json.loads(volume_file.read_text(encoding="utf-8"))
        assert content == volume_data, "Existing volume file should not be overwritten"
        print("✓ test_seed_does_not_overwrite_existing passed")


def test_seed_creates_parent_dir():
    """A volume whose parent does not exist yet must still be seeded."""
    with tempfile.TemporaryDirectory() as tmpdir:
        tmp_path = Path(tmpdir)
        packaged = tmp_path / "packaged.json"
        packaged.write_text(json.dumps({"en": {}}), encoding="utf-8")
        volume = tmp_path / "does" / "not" / "exist" / "sop_books.json"

        seed_book_titles(volume_path=volume, packaged_path=packaged)
        assert volume.is_file(), "seed must create missing parent directories"
        print("✓ test_seed_creates_parent_dir passed")


def test_seed_rejects_invalid_json():
    """A corrupt packaged table must not be copied onto the volume."""
    with tempfile.TemporaryDirectory() as tmpdir:
        tmp_path = Path(tmpdir)
        packaged = tmp_path / "packaged.json"
        packaged.write_text("{ this is not json", encoding="utf-8")
        volume = tmp_path / "sop_books.json"

        seed_book_titles(volume_path=volume, packaged_path=packaged)
        assert not volume.exists(), "invalid JSON must not be seeded"
        print("✓ test_seed_rejects_invalid_json passed")


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


if __name__ == "__main__":
    test_seed_creates_file_when_missing()
    test_seed_does_not_overwrite_existing()
    test_seed_creates_parent_dir()
    test_seed_rejects_invalid_json()
    test_ensure_data_dirs()
    print("\n✓ All tests passed!")
