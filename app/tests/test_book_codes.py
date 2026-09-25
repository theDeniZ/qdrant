"""Tests for `app.book_codes` — the sopack book-code registry exporter.

Run: cd qdrant && /workspaces/sdarm/.venv/bin/python3.11 -m unittest app.tests.test_book_codes
"""

from __future__ import annotations

import json
import sys
import tempfile
import unittest
from pathlib import Path

# Run standalone as well as `-m app.tests.test_book_codes`: executing the
# file directly puts app/tests/ on sys.path, not the repo root.
sys.path.insert(0, str(Path(__file__).resolve().parents[2]))

from app.book_codes import SCHEMA, build_registry, export  # noqa: E402


SAMPLE_SOP_BOOKS = {
    "en": {
        "SC": {"titles": ["Steps to Christ"]},
        "GC": {"titles": ["The Great Controversy"], "year": 1888},
        "ATNW": {"titles": ["The Atonement"], "author": "J. H. Waggoner",
                 "year": 1884, "corpus": "pioneers"},
    },
    "de": {
        "BW": {"titles": ["Der bessere Weg zu einem neuen Leben"], "en_code": "SC"},
        "DGK": {"titles": ["Der große Konflikt"], "en_code": "GC"},
        # Same literal code as an English one, to exercise the collapse path.
        "SC": {"titles": ["Schritte zu Christus (dt.)"], "en_code": "SC"},
    },
}


class BuildRegistryTests(unittest.TestCase):
    def test_distinct_codes_become_separate_rows(self):
        codes = build_registry(SAMPLE_SOP_BOOKS)
        self.assertIn("SC", codes)
        self.assertIn("BW", codes)
        self.assertIn("DGK", codes)
        self.assertIn("ATNW", codes)
        # BW (German-only code) never appears in the English table.
        self.assertEqual(codes["BW"]["lang"], ["de"])

    def test_same_literal_code_across_languages_collapses_to_one_row(self):
        codes = build_registry(SAMPLE_SOP_BOOKS)
        self.assertEqual(codes["SC"]["lang"], ["de", "en"])

    def test_english_title_preferred_when_code_recorded_in_both(self):
        codes = build_registry(SAMPLE_SOP_BOOKS)
        # SC is recorded under both en and de; en is processed first, so its
        # title wins even though "de" sorts before "en" alphabetically.
        self.assertEqual(codes["SC"]["title"], "Steps to Christ")

    def test_de_only_code_keeps_its_own_title(self):
        codes = build_registry(SAMPLE_SOP_BOOKS)
        self.assertEqual(codes["BW"]["title"], "Der bessere Weg zu einem neuen Leben")

    def test_corpus_passthrough(self):
        codes = build_registry(SAMPLE_SOP_BOOKS)
        self.assertEqual(codes["ATNW"]["corpus"], "pioneers")
        # A plain EGW code carries no corpus.
        self.assertIsNone(codes["GC"]["corpus"])

    def test_slug_is_null_when_sop_books_has_no_slug_field(self):
        codes = build_registry(SAMPLE_SOP_BOOKS)
        self.assertIsNone(codes["SC"]["slug"])
        self.assertIsNone(codes["ATNW"]["slug"])

    def test_slug_passthrough_when_present(self):
        sop_books = {"en": {"ATNW": {"titles": ["The Atonement"],
                                      "slug": "waggoner-jh__the-atonement"}}}
        codes = build_registry(sop_books)
        self.assertEqual(codes["ATNW"]["slug"], "waggoner-jh__the-atonement")

    def test_empty_code_key_is_skipped(self):
        sop_books = {"en": {"": {"titles": ["nothing"]}}}
        codes = build_registry(sop_books)
        self.assertEqual(codes, {})

    def test_missing_titles_list_does_not_crash(self):
        sop_books = {"en": {"XYZ": {}}}
        codes = build_registry(sop_books)
        self.assertIsNone(codes["XYZ"]["title"])

    def test_unrecognised_language_is_still_included(self):
        sop_books = {"ru": {"RUCODE": {"titles": ["Заголовок"]}}}
        codes = build_registry(sop_books)
        self.assertEqual(codes["RUCODE"]["lang"], ["ru"])


class ExportTests(unittest.TestCase):
    def test_export_writes_expected_shape(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            tmp = Path(tmpdir)
            sop_books_path = tmp / "sop_books.json"
            sop_books_path.write_text(json.dumps(SAMPLE_SOP_BOOKS), encoding="utf-8")
            out_path = tmp / "nested" / "book_codes.json"

            registry = export(sop_books_path, out_path, generated_at="2026-09-24T00:00:00Z")

            self.assertEqual(registry["schema"], SCHEMA)
            self.assertEqual(registry["generated_from"], str(sop_books_path))
            self.assertEqual(registry["generated_at"], "2026-09-24T00:00:00Z")
            self.assertIn("ATNW", registry["codes"])

            # Creates missing parent directories.
            self.assertTrue(out_path.is_file())
            on_disk = json.loads(out_path.read_text(encoding="utf-8"))
            self.assertEqual(on_disk, registry)

    def test_export_default_generated_at_is_utc_z_suffixed(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            tmp = Path(tmpdir)
            sop_books_path = tmp / "sop_books.json"
            sop_books_path.write_text(json.dumps(SAMPLE_SOP_BOOKS), encoding="utf-8")
            out_path = tmp / "book_codes.json"

            registry = export(sop_books_path, out_path)
            self.assertTrue(registry["generated_at"].endswith("Z"))

    def test_export_is_idempotent_and_fully_recomputed(self):
        # Rewriting must not accumulate stale codes from a previous run —
        # the whole point (docstring) is that this file is never hand-edited.
        with tempfile.TemporaryDirectory() as tmpdir:
            tmp = Path(tmpdir)
            sop_books_path = tmp / "sop_books.json"
            out_path = tmp / "book_codes.json"

            sop_books_path.write_text(json.dumps({"en": {"A": {"titles": ["A"]}}}),
                                       encoding="utf-8")
            export(sop_books_path, out_path)

            sop_books_path.write_text(json.dumps({"en": {"B": {"titles": ["B"]}}}),
                                       encoding="utf-8")
            registry = export(sop_books_path, out_path)

            self.assertNotIn("A", registry["codes"])
            self.assertIn("B", registry["codes"])


if __name__ == "__main__":
    unittest.main()
