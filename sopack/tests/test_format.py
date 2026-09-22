"""Tests for the .sopack container itself (sopack/format.py).

The concurrency cases here are regressions, not hypotheticals: a real 40-minute
embed of a 2486-block book was lost when a second writer aimed at the same
output path removed the first one's vector spool on its way down.
"""

from __future__ import annotations

import glob
import os
import random
import sys
import tempfile
import unittest
import zipfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))

from sopack import contract  # noqa: E402
from sopack.format import (ITEM_SIZE, PackError, PackReader,  # noqa: E402
                           PackWriter, VECTORS)

DIM = contract.VECTOR_SIZE


def _payload(i: int) -> dict:
    return {"lang": "en", "book_code": "TT", "book_pair": "TT", "page": 1,
            "para": i + 1, "para_key": f"1.{i + 1}",
            "raw_text": f"paragraph {i}", "aligned": None}


def _vec(rng) -> list[float]:
    return [rng.random() for _ in range(DIM)]


def _finish(w, rng, n_probe=1):
    w.set_books([{"book_code": "TT", "lang": "en", "points": w.count}])
    w.set_probe([{"id": f"canary-{i}", "collection": "sop"} for i in range(n_probe)],
                [_vec(rng) for _ in range(n_probe)])


class TestRoundTrip(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.mkdtemp()
        self.out = Path(self.dir) / "t.sopack"
        self.rng = random.Random(7)

    def test_round_trip_preserves_vectors_and_order(self):
        vectors = []
        with PackWriter(self.out, profile="sop", pack_id="p", created_by="t") as w:
            for i in range(300):
                v = _vec(self.rng)
                vectors.append(v)
                w.add(f"en:TT:1.{i + 1}#0", _payload(i), v)
            _finish(w, self.rng)

        r = PackReader(self.out)
        self.assertEqual(r.check(), [])
        self.assertEqual(r.count, 300)
        got = [v for _, vs in r.batches(size=64) for v in vs]
        self.assertEqual(len(got), 300)
        # float32 storage, so exact equality is not expected — but the error
        # must be float32 rounding, not a reordering or an off-by-one read.
        worst = max(abs(a - b) for va, vb in zip(vectors, got) for a, b in zip(va, vb))
        self.assertLess(worst, 1e-6)
        r.close()

    def test_vectors_are_stored_not_deflated(self):
        """Deflating normalised floats buys ~2 % and costs the SERVER a full
        decompression pass over every byte at import time."""
        with PackWriter(self.out, profile="sop", pack_id="p", created_by="t") as w:
            w.add("en:TT:1.1#0", _payload(0), _vec(self.rng))
            _finish(w, self.rng)
        info = zipfile.ZipFile(self.out).getinfo(VECTORS)
        self.assertEqual(info.compress_type, zipfile.ZIP_STORED)
        self.assertEqual(info.file_size, DIM * ITEM_SIZE)

    def test_wrong_dimension_rejected(self):
        with self.assertRaises(PackError):
            with PackWriter(self.out, profile="sop", pack_id="p", created_by="t") as w:
                w.add("en:TT:1.1#0", _payload(0), [0.0] * 8)

    def test_bad_payload_rejected(self):
        bad = _payload(0)
        del bad["aligned"]
        with self.assertRaises(PackError):
            with PackWriter(self.out, profile="sop", pack_id="p", created_by="t") as w:
                w.add("en:TT:1.1#0", bad, _vec(self.rng))


class TestIntegrityGuards(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.mkdtemp()
        self.out = Path(self.dir) / "t.sopack"
        self.rng = random.Random(11)
        with PackWriter(self.out, profile="sop", pack_id="p", created_by="t") as w:
            for i in range(10):
                w.add(f"en:TT:1.{i + 1}#0", _payload(i), _vec(self.rng))
            _finish(w, self.rng)

    def test_reading_refuses_before_check(self):
        """Editing raw_text changes neither uid nor id, so the id check cannot
        see it — only the manifest sha256 can. Reading therefore refuses until
        check() has passed, making the safe order the only available order."""
        r = PackReader(self.out)
        with self.assertRaises(PackError):
            next(r.batches(size=4))
        self.assertEqual(r.check(), [])
        self.assertEqual(len(next(r.batches(size=4))[0]), 4)
        r.close()

    def test_tampered_payload_detected_by_checksum(self):
        src = zipfile.ZipFile(self.out)
        lines = src.read("points.jsonl").decode().split("\n")
        lines[0] = lines[0].replace("paragraph 0", "TAMPERED!!")
        tampered = Path(self.dir) / "bad.sopack"
        out = zipfile.ZipFile(tampered, "w")
        for name in src.namelist():
            data = "\n".join(lines).encode() if name == "points.jsonl" else src.read(name)
            zi = zipfile.ZipInfo(name)
            zi.compress_type = src.getinfo(name).compress_type
            out.writestr(zi, data)
        out.close()
        src.close()

        r = PackReader(tampered)
        errors = r.check()
        self.assertTrue(any("points.jsonl" in e and "sha256" in e for e in errors), errors)
        with self.assertRaises(PackError):
            next(r.batches(size=4))
        r.close()


class TestConcurrentWriters(unittest.TestCase):
    """Regression: two writers aimed at the same output must not destroy each
    other. Both the vector spool and the output file were previously derived
    from the destination path, so the loser's cleanup unlinked the winner's
    work — a real, 40-minute data loss."""

    def setUp(self):
        self.dir = tempfile.mkdtemp()
        self.out = Path(self.dir) / "same.sopack"
        self.rng = random.Random(13)

    def _temps(self):
        return glob.glob(os.path.join(self.dir, ".same.sopack.*"))

    def test_failing_writer_does_not_destroy_a_live_one(self):
        a = PackWriter(self.out, profile="sop", pack_id="A", created_by="t").__enter__()
        for i in range(5):
            a.add(f"en:TT:1.{i + 1}#0", _payload(i), _vec(self.rng))

        b = PackWriter(self.out, profile="sop", pack_id="B", created_by="t").__enter__()
        self.assertNotEqual(a._spool_path, b._spool_path)
        self.assertNotEqual(a._tmp_out, b._tmp_out)
        b.__exit__(RuntimeError, RuntimeError("boom"), None)
        self.assertTrue(a._spool_path.exists(), "B's cleanup removed A's spool")

        for i in range(5, 10):
            a.add(f"en:TT:1.{i + 1}#0", _payload(i), _vec(self.rng))
        _finish(a, self.rng)
        a.__exit__(None, None, None)

        r = PackReader(self.out)
        self.assertEqual(r.check(), [])
        self.assertEqual(r.count, 10)
        self.assertEqual(r.manifest["pack_id"], "A")
        r.close()
        self.assertEqual(self._temps(), [], "temp files were left behind")

    def test_failed_run_leaves_previous_pack_intact(self):
        with PackWriter(self.out, profile="sop", pack_id="GOOD", created_by="t") as w:
            w.add("en:TT:1.1#0", _payload(0), _vec(self.rng))
            _finish(w, self.rng)

        with self.assertRaises(RuntimeError):
            with PackWriter(self.out, profile="sop", pack_id="DOOMED", created_by="t") as w:
                w.add("en:TT:1.1#0", _payload(0), _vec(self.rng))
                raise RuntimeError("die mid-pack")

        r = PackReader(self.out)
        self.assertEqual(r.manifest["pack_id"], "GOOD")
        self.assertEqual(r.check(), [])
        r.close()
        self.assertEqual(self._temps(), [])

    def test_destination_never_holds_a_partial_pack(self):
        """The output is renamed into place only once it is complete, so a
        reader can never observe a half-written pack at the destination."""
        w = PackWriter(self.out, profile="sop", pack_id="P", created_by="t").__enter__()
        for i in range(5):
            w.add(f"en:TT:1.{i + 1}#0", _payload(i), _vec(self.rng))
        self.assertFalse(self.out.exists(), "destination existed before completion")
        _finish(w, self.rng)
        w.__exit__(None, None, None)
        self.assertTrue(self.out.exists())


if __name__ == "__main__":
    unittest.main(verbosity=2)
