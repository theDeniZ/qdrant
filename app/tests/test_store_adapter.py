"""Round-trip acceptance test (SOPACK-AUTONOMY.md §5.4/§5.5): a real
``sopack/2`` pack, written by ``sopack.pack.pack``, read back and run through
the SAME ``import_service.run_calibration_probe`` function against **two**
different :class:`app.store_adapter.StoreAdapter` implementations — the
Qdrant-shaped fake HTTP server and a plain-Python in-memory store. Neither
``sopack/`` nor ``run_calibration_probe`` itself changes between the two: the
same pack, the same code, a different backend — proving the store-neutral
design rather than merely asserting it.

Also covers the empty-store contract-fingerprint path (SOPACK-2-FORMAT.md §4
step 3) and its refusal under a mismatched fingerprint.

Run: .venv/bin/python3.11 -m app.tests.test_store_adapter
"""

from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))

from sopack.tests import _helpers  # noqa: E402

with _helpers.patched_book_module() as _book_mod:
    from sopack import pack as pack_mod  # noqa: E402

from sopack import contract  # noqa: E402
from sopack.format import PackReader  # noqa: E402

from app import import_service, store_adapter  # noqa: E402
from app.tests.fake_qdrant import FakeQdrant  # noqa: E402

DIM = contract.VECTOR_SIZE


def _build_v2_pack(out_path, *, fixture_texts=("fixture a", "fixture b")):
    """A real, checked ``sopack/2`` pack, built exactly the way
    ``sopack.pack.pack`` builds one in production — just with a
    ``FakeTextEmbedding`` in place of the real 2 GB model, and a calibration
    fixture whose vectors are precomputed to match it (see
    ``_helpers.fake_calibration``)."""
    book = _helpers.make_book(_book_mod, lang="en", book_code="RTT", n_blocks=3,
                              title="Round Trip Test", author="Tester", year=2020,
                              corpus="pioneers", slug="round-trip-test")
    calibration = _helpers.fake_calibration(list(fixture_texts), dim=DIM, profile="sop")
    with mock.patch.object(pack_mod, "TextEmbedding",
                           lambda model_name=None, **kw: _helpers.FakeTextEmbedding(
                               model_name, dim=DIM)):
        pack_mod.pack([(book, None)], out_path, calibration=calibration,
                     workers=1, progress=lambda *_: None)
    return calibration


class RoundTripAcceptanceTests(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.out = Path(self._tmp.name) / "round-trip.sopack"
        self.calibration = _build_v2_pack(self.out)
        self.profile = contract.get_profile("sop")
        self.collection = store_adapter.collection_for("sop")

    def _open_checked(self) -> PackReader:
        reader = PackReader(self.out)
        errors = reader.check()
        self.assertEqual(errors, [], f"pack failed check(): {errors}")
        return reader

    # ── the acceptance case itself: same pack, in-memory backend ────────────

    def test_probe_and_full_import_succeed_against_the_in_memory_adapter(self):
        adapter = store_adapter.InMemoryAdapter()
        adapter.seed_collection(self.collection, dim=DIM)

        with self._open_checked() as reader:
            # 1. the probe (SOPACK-2-FORMAT.md §4) — empty store, so this is
            # the step-3 fingerprint path. `fixture=` is the SAME fixture the
            # pack was calibrated against (setUp's `_build_v2_pack`) — in
            # production this is always the committed file (fixture=None);
            # here it stands in for "the importer's own copy agrees".
            detail = import_service.run_calibration_probe(
                reader, adapter, self.profile, self.collection, fixture=self.calibration)
            self.assertIn("empty store", detail)
            self.assertEqual(adapter.get_fingerprint(self.collection), contract.CONTRACT_SHA256)

            # 2. the actual import: every point, upserted through the SAME
            # adapter interface the Qdrant path uses.
            total = 0
            for points, vectors in reader.batches(size=64):
                body = [{"id": p["id"], "payload": p["payload"], "vector": v}
                       for p, v in zip(points, vectors)]
                adapter.upsert(self.collection, body)
                total += len(points)

        self.assertEqual(total, reader.count)
        self.assertEqual(len(adapter.points(self.collection)), reader.count)
        # Spot-check one point round-trips with its payload and vector intact.
        with self._open_checked() as reader2:
            first_points, first_vectors = next(reader2.batches(size=1))
        pid = first_points[0]["id"]
        [row] = adapter.retrieve(self.collection, [pid], with_payload=True, with_vector=True)
        self.assertEqual(row["payload"], first_points[0]["payload"])
        self.assertEqual(row["vector"], first_vectors[0])

    def test_probe_succeeds_against_the_qdrant_shaped_fake_too(self):
        """The other half of "two backends, one pack": the same pack passes
        the same probe against the Qdrant-shaped adapter (FakeQdrant over real
        HTTP), not just the in-memory one."""
        qdrant = FakeQdrant()
        self.addCleanup(qdrant.stop)
        qdrant.create_collection(self.collection, store_adapter.QDRANT_VECTOR_NAME, DIM,
                                 store_adapter.QDRANT_DISTANCE)
        adapter = store_adapter.QdrantAdapter(
            qdrant.url, fingerprint_path=str(Path(self._tmp.name) / "fp.json"))

        with self._open_checked() as reader:
            detail = import_service.run_calibration_probe(
                reader, adapter, self.profile, self.collection, fixture=self.calibration)
        self.assertIn("empty store", detail)
        self.assertEqual(adapter.get_fingerprint(self.collection), contract.CONTRACT_SHA256)

    # ── step 2: fixture <-> store, using the REAL committed fixture ─────────

    def test_probe_step2_passes_when_the_store_already_holds_fixture_points(self):
        """Seeds the in-memory store with one of THIS pack's own fixture
        entries under its id and (exact) fixture vector — exactly what a live
        Qdrant collection already holds in production, where the same
        committed fixture both calibrated the pack and was previously
        imported into the collection."""
        entry = self.calibration["entries"][0]
        adapter = store_adapter.InMemoryAdapter()
        adapter.seed_collection(self.collection, dim=DIM)
        adapter.put_point(self.collection, entry["id"], {"lang": entry.get("lang")},
                          entry["vector"])

        with self._open_checked() as reader:
            detail = import_service.run_calibration_probe(
                reader, adapter, self.profile, self.collection, fixture=self.calibration)
        self.assertIn("fixture<->store", detail)
        self.assertNotIn("empty store", detail)

    # ── failure paths ────────────────────────────────────────────────────

    def test_refuses_a_pack_calibrated_against_a_different_fixture(self):
        other_fixture = _helpers.fake_calibration(["totally different fixture text"], dim=DIM)
        out = Path(self._tmp.name) / "wrong-fixture.sopack"
        book = _helpers.make_book(_book_mod, lang="en", book_code="WF", n_blocks=1)
        with mock.patch.object(pack_mod, "TextEmbedding",
                               lambda model_name=None, **kw: _helpers.FakeTextEmbedding(
                                   model_name, dim=DIM)):
            pack_mod.pack([(book, None)], out, calibration=other_fixture,
                         workers=1, progress=lambda *_: None)

        adapter = store_adapter.InMemoryAdapter()
        adapter.seed_collection(self.collection, dim=DIM)
        with PackReader(out) as reader:
            reader.check()
            # The importer's fixture here is `self.calibration` (setUp's) —
            # deliberately NOT `other_fixture`, which is what the pack above
            # was calibrated against, so the shas disagree.
            with self.assertRaises(import_service.StageFailure) as ctx:
                import_service.run_calibration_probe(reader, adapter, self.profile,
                                                      self.collection, fixture=self.calibration)
        self.assertIn("different fixture", str(ctx.exception))

    def test_refuses_when_the_store_fingerprint_does_not_match(self):
        adapter = store_adapter.InMemoryAdapter()
        adapter.seed_collection(self.collection, dim=DIM)
        adapter.set_fingerprint(self.collection, "0" * 64)  # a different contract entirely

        with self._open_checked() as reader:
            with self.assertRaises(import_service.StageFailure) as ctx:
                import_service.run_calibration_probe(reader, adapter, self.profile,
                                                      self.collection, fixture=self.calibration)
        self.assertIn("different contract", str(ctx.exception))


if __name__ == "__main__":
    unittest.main(verbosity=2)
