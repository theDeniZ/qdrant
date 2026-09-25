#!/usr/bin/env python3
"""Direction 2 of the M2 pack conformance leg (SOPACK-1.0-PLAN.md §4):
a Rust-written `sopack/2` pack, opened and verified by the real Python
`sopack.format.PackReader` + `sopack.verify.verify`.

The pack itself (``rust_v2_sample.sopack``) is written by
`cargo run -p sopack-format --example make_v2_conformance_pack` — run that
again first if you have changed the writer and want a fresh artifact.

Checks:
  1. ``PackReader(path).check()`` is clean (schema, embedding/target/contract
     vs. the *Python* contract loader, sha256 of every entry, byte counts).
  2. ``PackReader.batches()`` streams every point with id verification
     enabled — recomputes each id from its uid via the Python contract and
     compares.
  3. ``sopack.verify.verify(path)`` — the full offline pass (payload
     validation, per-book counts, probe shape) — is clean.

Exit code 0 and "OK" on success; non-zero and a description of every
mismatch otherwise.

Run:
    PYTHONPATH=/workspaces/sdarm/qdrant /workspaces/sdarm/.venv/bin/python3.11 \\
        qdrant/sopack-rs/conformance/packs/check_pack_py.py
"""

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[3]))  # .../qdrant
from sopack import verify as verify_mod  # noqa: E402
from sopack.format import PackReader  # noqa: E402

PACK_PATH = Path(__file__).with_name("rust_v2_sample.sopack")


def main() -> int:
    # Optional override so this same script also checks a real, model-embedded
    # pack (e.g. the M4 integration agent's `sopack pack` output on
    # `qdrant/packs/wdys.book.json`) without duplicating the checks — the
    # default (no argv) stays the M2 `rust_v2_sample.sopack` leg.
    pack_path = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else PACK_PATH
    if not pack_path.exists():
        print(
            f"FAIL: {pack_path} does not exist — run "
            "`cargo run -p sopack-format --example make_v2_conformance_pack` first "
            "(or pass a pack path explicitly)",
            file=sys.stderr,
        )
        return 1

    problems: list[str] = []

    with PackReader(pack_path) as reader:
        print(f"schema={reader.manifest.get('schema')!r} profile={reader.manifest.get('profile')!r} "
              f"points={reader.count} dim={reader.dim}")
        check_errors = reader.check()
        if check_errors:
            problems.append("PackReader.check() reported:\n  " + "\n  ".join(check_errors))
        else:
            seen = 0
            try:
                for points, vectors in reader.batches():
                    if len(points) != len(vectors):
                        problems.append(f"batch mismatch: {len(points)} points, {len(vectors)} vectors")
                    seen += len(points)
            except Exception as exc:  # noqa: BLE001 — report, don't crash the script
                problems.append(f"batches() raised: {exc}")
            if seen != reader.count:
                problems.append(f"streamed {seen} points, manifest says {reader.count}")
            else:
                print(f"batches(): streamed {seen} points with id verification, all matched")

    verify_errors = verify_mod.verify(pack_path)
    if verify_errors:
        problems.append("sopack.verify.verify() reported:\n  " + "\n  ".join(verify_errors))
    else:
        print("sopack.verify.verify(): clean")

    if problems:
        print("FAIL:\n" + "\n".join(problems), file=sys.stderr)
        return 1

    print("OK — Rust-written sopack/2 pack reads and verifies cleanly with the Python reference")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
