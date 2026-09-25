"""Tests for app/calibration.py's export command, against the in-process
fake Qdrant (app/tests/fake_qdrant.py) — no network, no real tokenizer file.

Run: .venv/bin/python3.11 -m app.tests.test_calibration
"""

from __future__ import annotations

import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))

from app import calibration  # noqa: E402
from app.tests.fake_qdrant import FakeQdrant  # noqa: E402

DIM = 8
VECTOR_NAME = "fast-multilingual-e5-large"


class _FakeTokenizer:
    """Deterministic word-count stand-in for the real tokenizers.Tokenizer —
    avoids depending on a real tokenizer.json file in this unit test. Every
    space-separated word counts as ~1.3 tokens, close enough to exercise the
    ">max_tokens" branch without the real vocabulary."""

    class _Ids(list):
        pass

    def encode(self, text: str):
        n = max(1, int(len(text.split()) * 1.3))
        ids = self._Ids(range(n))
        return type("Enc", (), {"ids": ids})()


def _vec(seed: int) -> list[float]:
    import random
    rnd = random.Random(seed)
    return [rnd.uniform(-1.0, 1.0) for _ in range(DIM)]


class CalibrationExportTests(unittest.TestCase):
    def setUp(self):
        self.qdrant = FakeQdrant()
        self.addCleanup(self.qdrant.stop)
        self.qdrant.create_collection("sop", VECTOR_NAME, DIM)
        self.qdrant.create_collection("bibles", VECTOR_NAME, DIM)

        # EGW points (no `corpus` key) in several languages.
        n = 0
        for lang in ("en", "de", "es", "fr", "it", "ja", "pt"):
            for i in range(3):
                n += 1
                self.qdrant.put_point(
                    "sop", f"egw-{lang}-{i}",
                    {"lang": lang, "book_code": "AG", "raw_text": f"EGW {lang} text {i}"},
                    _vec(n))

        # Pioneer points (carry `corpus`) — two short ones (what the "pioneer"
        # category itself picks, `_PIONEER_WANTED=2`) plus a third, longer one
        # that only the dedicated ">max_tokens" scan should reach (it must
        # NOT collide with the two already claimed by the pioneer category).
        self.qdrant.put_point(
            "sop", "pioneer-short",
            {"lang": "en", "book_code": "WDYS", "corpus": "pioneers",
             "raw_text": "a short pioneer paragraph"}, _vec(900))
        self.qdrant.put_point(
            "sop", "pioneer-medium",
            {"lang": "en", "book_code": "WDYS", "corpus": "pioneers",
             "raw_text": "a slightly longer but still short pioneer paragraph"}, _vec(902))
        long_text = " ".join(["word"] * 500)  # ~650 fake tokens, over 512
        self.qdrant.put_point(
            "sop", "pioneer-long",
            {"lang": "en", "book_code": "WDYS", "corpus": "pioneers", "raw_text": long_text},
            _vec(901))

        for i, bible in enumerate(("kjv", "luther1912", "schlachter", "synodal", "korean")):
            self.qdrant.put_point(
                "bibles", f"bible-{bible}",
                {"bible": bible, "osis": "Gen.1.1", "text": f"In the beginning ({bible})"},
                _vec(1000 + i))

        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)

    def _build(self):
        with mock.patch.object(calibration, "_tokenizer", lambda model_dir: _FakeTokenizer()):
            return calibration.build_fixture(
                self.qdrant.url, model_dir="unused", vector_name=VECTOR_NAME)

    def test_fixture_has_16_entries_covering_every_category(self):
        fixture = self._build()
        self.assertEqual(fixture["schema"], "sopack.calibration/1")
        entries = fixture["entries"]
        self.assertEqual(len(entries), 16)

        notes = [e["note"] for e in entries]
        self.assertTrue(any("EGW en" in n for n in notes))
        self.assertTrue(any("EGW de" in n for n in notes))
        other_lang_notes = [n for n in notes if n.startswith("EGW ") and
                            n.split()[1] not in ("en", "de")]
        self.assertGreaterEqual(len(other_lang_notes), 1)
        self.assertTrue(any("pioneer" in n and "corpus" in n for n in notes))
        self.assertTrue(any(">" in n and "tokens" in n for n in notes))
        bible_notes = [n for n in notes if n.startswith("bible ")]
        self.assertGreaterEqual(len(bible_notes), 1)

        # Every entry is well-formed and carries a real vector.
        for e in entries:
            self.assertIn(e["profile"], ("sop", "bible"))
            self.assertEqual(len(e["vector"]), DIM)
            self.assertTrue(e["text"])

    def test_long_entry_exceeds_the_token_threshold(self):
        fixture = self._build()
        long_entries = [e for e in fixture["entries"] if "pioneer-long" in e["id"]]
        self.assertEqual(len(long_entries), 1)

    def test_selection_is_deterministic_across_reruns(self):
        first = self._build()
        second = self._build()
        self.assertEqual([e["id"] for e in first["entries"]],
                         [e["id"] for e in second["entries"]])

    def test_export_writes_file_and_reports_sha256(self):
        out = Path(self._tmp.name) / "calibration.json"
        with mock.patch.object(calibration, "_tokenizer", lambda model_dir: _FakeTokenizer()):
            written = calibration.export(self.qdrant.url, out, model_dir="unused",
                                          vector_name=VECTOR_NAME)
        self.assertEqual(written, out)
        doc = json.loads(out.read_text(encoding="utf-8"))
        self.assertEqual(len(doc["entries"]), 16)
        sha = calibration.sha256_of(out)
        self.assertEqual(len(sha), 64)


if __name__ == "__main__":
    unittest.main(verbosity=2)
