"""Offline verification of a ``.sopack`` — no Qdrant, no fastembed.

``sopack verify`` is meant to run anywhere, including CI: it re-checks every
byte the pack claims (``PackReader.check()``) and then does a full streaming
pass counting points per book, so a truncated ``vectors.f32`` or a manifest
whose declared per-book counts drifted from what was actually written is
caught before anyone uploads the file.
"""

from __future__ import annotations

import collections

from .format import PackError, PackReader

__all__ = ["verify"]


def verify(pack_path) -> list[str]:
    """Every problem found with *pack_path*. Empty list means it is clean."""
    errors: list[str] = []
    try:
        reader = PackReader(pack_path)
    except (PackError, OSError) as exc:
        return [str(exc)]

    with reader:
        errors.extend(reader.check())
        if errors:
            # check() failed — points.jsonl/vectors.f32 cannot be trusted, so
            # batches() would refuse to stream anyway (or stream tampered data).
            return errors

        try:
            profile = reader.profile
        except ValueError as exc:
            return [str(exc)]

        manifest_books = {b["book_code"]: b for b in reader.manifest.get("books", [])}
        counts: dict[str, int] = collections.Counter()
        total = 0
        try:
            for points, vectors in reader.batches():
                if len(points) != len(vectors):
                    errors.append(
                        f"batch mismatch: {len(points)} points, {len(vectors)} vectors")
                for rec in points:
                    payload = rec.get("payload") or {}
                    problems = _payload_problems(profile, payload)
                    if problems:
                        errors.append(f"{rec.get('uid')}: " + "; ".join(problems))
                    code = payload.get(profile.identity, "?")
                    counts[code] += 1
                    total += 1
        except PackError as exc:
            errors.append(str(exc))
            return errors

        declared_total = reader.count
        if total != declared_total:
            errors.append(
                f"streamed {total} points, manifest counts.points says {declared_total}")

        for code, entry in manifest_books.items():
            got = counts.get(code, 0)
            want = entry.get("points")
            if want is not None and got != want:
                errors.append(
                    f"book {code!r}: manifest declares {want} points, stream has {got}")
        for code in counts:
            if code not in manifest_books:
                errors.append(
                    f"book {code!r} has {counts[code]} points but no entry in manifest.books")

        probe_vectors = reader.probe_vectors()
        probe = reader.manifest.get("probe") or {}
        # sopack/1 declared canaries under "canaries"; sopack/2 declares
        # calibration-fixture entries under "entries" (SOPACK-2-FORMAT.md §2).
        probe_meta = probe.get("entries") if reader.manifest.get("schema") != "sopack/1" \
            else probe.get("canaries")
        probe_meta = probe_meta or []
        if probe_meta and len(probe_vectors) != len(probe_meta):
            errors.append(
                f"probe: manifest declares {len(probe_meta)} probe entr(y/ies), "
                f"probe.f32 holds {len(probe_vectors)} vectors")
        for v in probe_vectors:
            if len(v) != reader.dim:
                errors.append(f"probe vector has {len(v)} dims, expected {reader.dim}")
                break

    return errors


def _payload_problems(profile, payload: dict) -> list[str]:
    problems = []
    for key in profile.required:
        if key not in payload:
            problems.append(f"missing required payload key {key!r}")
    text = payload.get(profile.text_field)
    if not isinstance(text, str) or not text.strip():
        problems.append(f"{profile.text_field!r} is empty")
    return problems
