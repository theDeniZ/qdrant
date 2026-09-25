#!/usr/bin/env python3
"""M4 integration check (c): a real, model-embedded `sopack/2` pack built by
the Rust `sopack pack` (e.g. on `qdrant/packs/wdys.book.json`) passes the
Python importer's own `run_calibration_probe` — the same function
`app/import_service.py` runs against a live Qdrant collection, here run
against `app.store_adapter.InMemoryAdapter` (no network), exactly like
`app/tests/test_store_adapter.py`'s round-trip acceptance test does for a
Python-built pack (SOPACK-AUTONOMY.md §5.4/§5.5) — this script is the same
proof for a *Rust*-built one.

Uses `fixture=None` (the importer's own committed
`contracts/e5-large-v1/calibration.json`), since the Rust binary was built
against and calibrated with that exact same committed file — there is no
test-only fixture override here, unlike `test_store_adapter.py`'s synthetic
`FakeTextEmbedding` packs.

Exit code 0 and "OK" on success; non-zero and a description otherwise.

Run:
    PYTHONPATH=/workspaces/sdarm/qdrant /workspaces/sdarm/.venv/bin/python3.11 \\
        qdrant/sopack-rs/conformance/packs/check_import_probe_py.py <pack.sopack>
"""

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[3]))  # .../qdrant

from sopack import contract  # noqa: E402
from sopack.format import PackReader  # noqa: E402

from app import import_service, store_adapter  # noqa: E402


def main() -> int:
    if len(sys.argv) != 2:
        print(f"usage: {sys.argv[0]} <pack.sopack>", file=sys.stderr)
        return 2
    pack_path = Path(sys.argv[1]).resolve()
    if not pack_path.exists():
        print(f"FAIL: {pack_path} does not exist", file=sys.stderr)
        return 1

    with PackReader(pack_path) as reader:
        check_errors = reader.check()
        if check_errors:
            print("FAIL: PackReader.check() reported:\n  " + "\n  ".join(check_errors), file=sys.stderr)
            return 1
        print(
            f"schema={reader.manifest.get('schema')!r} profile={reader.manifest.get('profile')!r} "
            f"points={reader.count} dim={reader.dim}"
        )

        profile = contract.get_profile(reader.profile.name)
        collection = store_adapter.collection_for(profile.name)
        adapter = store_adapter.InMemoryAdapter()
        adapter.seed_collection(collection, dim=contract.VECTOR_SIZE)

        try:
            detail = import_service.run_calibration_probe(reader, adapter, profile, collection)
        except import_service.StageFailure as exc:
            print(f"FAIL: run_calibration_probe raised: {exc}", file=sys.stderr)
            return 1
        print(f"run_calibration_probe: {detail}")

        # The actual import, same as the round-trip acceptance test — proves
        # every point (not just the probe) is acceptable to the importer's
        # adapter interface, e.g. no payload the InMemoryAdapter chokes on.
        total = 0
        for points, vectors in reader.batches(size=64):
            body = [
                {"id": p["id"], "payload": p["payload"], "vector": v}
                for p, v in zip(points, vectors)
            ]
            adapter.upsert(collection, body)
            total += len(points)
        if total != reader.count:
            print(f"FAIL: imported {total} points, manifest says {reader.count}", file=sys.stderr)
            return 1
        if len(adapter.points(collection)) != reader.count:
            print(
                f"FAIL: adapter holds {len(adapter.points(collection))} points after import, "
                f"expected {reader.count}",
                file=sys.stderr,
            )
            return 1

    print(f"OK — {total} point(s) probed and imported cleanly via the in-memory adapter")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
