"""Unit tests for sopack.pack.

Runnable standalone:
    /workspaces/sdarm/.venv/bin/python3.11 -m sopack.tests.test_pack

Uses FakeTextEmbedding (deterministic, no model load) and a fake
``sopack.book`` seam (see _helpers.py) — never touches the real 2 GB model or
a real book.py.
"""

from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))

from sopack.tests import _helpers  # noqa: E402

# sopack.pack does `from .book import ...` at MODULE level (see its
# docstring — deliberate, so a missing fastembed fails at import time). This
# is the only place in this process sopack.pack is ever imported, so the fake
# only needs to be in sys.modules for this one import; patched_book_module()
# restores whatever was there afterward so it can't leak into test_book.py /
# test_extract.py's own run-time `from sopack.book import ...` calls.
with _helpers.patched_book_module() as _book_mod:
    from sopack import pack as pack_mod  # noqa: E402

from sopack import contract  # noqa: E402
from sopack.format import PackReader  # noqa: E402


def _calibration(n=2):
    return _helpers.fake_calibration([f"fixture text {i}" for i in range(n)])


class PackTests(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.out = Path(self._tmp.name) / "test.sopack"

    def _patch_embedder(self, dim=contract.VECTOR_SIZE):
        return mock.patch.object(
            pack_mod, "TextEmbedding",
            lambda model_name=None, **kw: _helpers.FakeTextEmbedding(model_name, dim=dim))

    def test_pack_read_round_trip(self):
        book1 = _helpers.make_book(_book_mod, lang="en", book_code="HSFD",
                                    n_blocks=3, title="History of the Sabbath",
                                    author="J. N. Andrews", year=1873, corpus="pioneers",
                                    slug="andrews-history-of-the-sabbath")
        book2 = _helpers.make_book(_book_mod, lang="en", book_code="GC",
                                    n_blocks=2, title="The Great Controversy",
                                    author="E. G. White", year=1888)
        calibration = _calibration(3)

        with self._patch_embedder():
            manifest = pack_mod.pack(
                [(book1, "sha-hsfd"), (book2, "sha-gc")], self.out,
                calibration=calibration,
                batch_size=2, workers=1, progress=lambda *_: None)

        self.assertEqual(manifest["counts"]["points"], 5)
        self.assertEqual(manifest["counts"]["books"], 2)
        self.assertEqual(manifest["counts"]["dim"], contract.VECTOR_SIZE)
        self.assertEqual(manifest["profile"], "sop")

        with PackReader(self.out) as reader:
            errors = reader.check()
            self.assertEqual(errors, [], f"pack failed check(): {errors}")
            total = 0
            seen_book_codes = set()
            for points, vectors in reader.batches(size=2):
                self.assertEqual(len(points), len(vectors))
                for rec, vec in zip(points, vectors):
                    self.assertEqual(len(vec), contract.VECTOR_SIZE)
                    self.assertIn("raw_text", rec["payload"])
                    seen_book_codes.add(rec["payload"]["book_code"])
                total += len(points)
            self.assertEqual(total, 5)
            self.assertEqual(seen_book_codes, {"HSFD", "GC"})

    def test_dimension_mismatch_rejected(self):
        book = _helpers.make_book(_book_mod, lang="en", book_code="X", n_blocks=2)
        with self._patch_embedder(dim=7):
            with self.assertRaises(pack_mod.PackBuildError) as ctx:
                pack_mod.pack([(book, None)], self.out, calibration=_calibration(1),
                              workers=1, progress=lambda *_: None)
        self.assertIn("dimension", str(ctx.exception))
        self.assertFalse(self.out.exists(), "no file should be left on a failed pack")

    def test_wrong_fastembed_version_rejected(self):
        book = _helpers.make_book(_book_mod, lang="en", book_code="X", n_blocks=1)
        with mock.patch.object(pack_mod.fastembed, "__version__", "0.5.1"):
            with self.assertRaises(pack_mod.PackBuildError) as ctx:
                pack_mod.pack([(book, None)], self.out, calibration=_calibration(1),
                              workers=1, progress=lambda *_: None)
        self.assertIn("0.5.1", str(ctx.exception))
        self.assertFalse(self.out.exists())

    def test_probe_written_and_readable(self):
        book = _helpers.make_book(_book_mod, lang="en", book_code="X", n_blocks=2)
        calibration = _calibration(4)
        with self._patch_embedder():
            manifest = pack_mod.pack([(book, None)], self.out, calibration=calibration,
                                      workers=1, progress=lambda *_: None)

        probe = manifest["probe"]
        self.assertIsNotNone(probe)
        self.assertEqual(probe["kind"], "calibration")
        self.assertEqual(probe["fixture_sha256"], contract.calibration_fixture_sha256(calibration))
        self.assertEqual(len(probe["entries"]), 4)
        got_ids = {e["id"] for e in probe["entries"]}
        self.assertEqual(got_ids, {e["id"] for e in calibration["entries"]})
        self.assertEqual(probe["self_check"]["n"], 4)
        self.assertGreaterEqual(probe["self_check"]["min_cosine"], contract.PACK_MIN_COSINE)
        self.assertEqual(probe["self_check"]["threshold"], contract.PACK_MIN_COSINE)

        with PackReader(self.out) as reader:
            reader.check()
            vectors = reader.probe_vectors()
            self.assertEqual(len(vectors), 4)
            for v in vectors:
                self.assertEqual(len(v), contract.VECTOR_SIZE)

    def test_titles_fragment_shape(self):
        de_book = _helpers.make_book(
            _book_mod, lang="de", book_code="BW", n_blocks=2,
            title="Der bessere Weg", book_pair="BW/SC")
        en_book = _helpers.make_book(
            _book_mod, lang="en", book_code="SC", n_blocks=2,
            title="Steps to Christ", author="E. G. White", year=1892)

        with self._patch_embedder():
            pack_mod.pack([(de_book, None), (en_book, None)], self.out,
                          calibration=_calibration(1),
                          workers=1, progress=lambda *_: None)

        with PackReader(self.out) as reader:
            titles = reader.titles()

        self.assertIsNotNone(titles)
        self.assertIn("de", titles)
        self.assertIn("en", titles)
        self.assertEqual(titles["de"]["BW"]["titles"], ["Der bessere Weg"])
        self.assertEqual(titles["de"]["BW"]["en_code"], "SC")
        self.assertEqual(titles["en"]["SC"]["titles"], ["Steps to Christ"])
        self.assertEqual(titles["en"]["SC"]["author"], "E. G. White")
        self.assertEqual(titles["en"]["SC"]["year"], 1892)
        self.assertNotIn("en_code", titles["en"]["SC"])  # English never gets one

    def test_no_calibration_entries_refused(self):
        book = _helpers.make_book(_book_mod, lang="en", book_code="X", n_blocks=1)
        with self._patch_embedder():
            with self.assertRaises(pack_mod.PackBuildError):
                pack_mod.pack([(book, None)], self.out,
                              calibration={"schema": "sopack.calibration/1", "entries": []},
                              workers=1, progress=lambda *_: None)

    def test_mixed_id_rule_refused(self):
        book1 = _helpers.make_book(_book_mod, lang="en", book_code="A", n_blocks=1,
                                    id_rule="sop/seq")
        book2 = _helpers.make_book(_book_mod, lang="en", book_code="B", n_blocks=1,
                                    id_rule="sop/plain")
        with self._patch_embedder():
            with self.assertRaises(pack_mod.PackBuildError) as ctx:
                pack_mod.pack([(book1, None), (book2, None)], self.out,
                              calibration=_calibration(1),
                              workers=1, progress=lambda *_: None)
        self.assertIn("id_rule", str(ctx.exception))

    def test_calibration_below_threshold_refused_before_any_book_embedded(self):
        """Acceptance criterion (SOPACK-AUTONOMY.md §5.3): pack fails fast,
        before embedding any book, when the calibration self-check fails —
        simulated here the same way a real pooling/library regression would
        show up: the fixture's committed vector no longer matches what this
        machine's embedder produces for that text."""
        book = _helpers.make_book(_book_mod, lang="en", book_code="X", n_blocks=1)
        calibration = _calibration(2)
        # Corrupt one fixture vector so it no longer matches FakeTextEmbedding's
        # deterministic output for that text — exactly what a drifted pooling
        # implementation would look like from the outside.
        calibration["entries"][0]["vector"] = [0.0] * contract.VECTOR_SIZE

        with self._patch_embedder():
            with self.assertRaises(pack_mod.CalibrationFailed) as ctx:
                pack_mod.pack([(book, None)], self.out, calibration=calibration,
                              workers=1, progress=lambda *_: None)
        self.assertIn("calibration self-check FAILED", str(ctx.exception))
        self.assertFalse(self.out.exists(), "no file should be left on a failed calibration")


if __name__ == "__main__":
    unittest.main()
