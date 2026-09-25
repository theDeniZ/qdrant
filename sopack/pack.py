"""``book.json`` (+ the committed calibration fixture) → ``.sopack`` — the
slow, Mac-only step.

This is the **only** module in the pipeline that imports an embedding
library, and it imports it **at module import time**, not inside a function.
That is deliberate (R7 / failures #2, #3): ``import sopack.pack`` is what the
CLI does the instant someone runs ``sopack pack``, so a missing or
mismatched ``fastembed`` fails in the first second, not twenty minutes into a
run. ``sopack.cli`` imports this module lazily, only for the ``pack``
subcommand, so every other command stays fastembed-free.

Against the seam ``sopack.book`` (owned by another agent — not defined by
this module, only imported): ``Book``, ``Block``, ``load``, ``to_payload``,
``uid``, ``BookError``. This module never inspects ``Book``/``Block``
internals beyond iterating ``book.blocks`` — every piece of per-book metadata
it needs (``lang``, ``book_code``, ``title``, ``author``, ``year``,
``corpus``, ``slug``, ``book_pair``) is read back out of the **payload**
``to_payload`` returns, because the ``sop``/``bible`` contract already
requires those fields on every point. That keeps this module decoupled from
whatever shape ``Book`` turns out to have.
"""

from __future__ import annotations

import hashlib
import json
import platform
import secrets
from datetime import datetime, timezone
from pathlib import Path

import fastembed
from fastembed import TextEmbedding

from . import __version__ as SOPACK_VERSION
from . import contract
from .book import Book, Block, BookError, load, to_payload, uid  # noqa: F401  (seam)
from .format import PackWriter

__all__ = ["pack", "PackBuildError", "CalibrationFailed"]


class PackBuildError(Exception):
    """The pack could not be built — a contract violation or bad input."""


class CalibrationFailed(PackBuildError):
    """The pack-time calibration self-check scored below
    ``contract.PACK_MIN_COSINE`` (SOPACK-AUTONOMY.md §3.1). Raised before any
    book is embedded — a broken environment fails in seconds, not twenty
    minutes into a real run."""


# ── preflight (R7) ───────────────────────────────────────────────────────────

def _preflight() -> None:
    """Everything this run will eventually need, checked before any slow work.

    ``import fastembed`` already happened at module level (see the module
    docstring); what is left is asserting it is the *right* fastembed —
    version and, transitively, pooling. This pin is a property of THIS
    Python reference implementation, not of the neutral contract (a Rust
    build has no fastembed at all) — acceptance of the vectors themselves is
    decided by the calibration self-check below, not by the library version;
    this check only turns a silently-wrong pooling into a fast, readable
    failure instead of a self-check that fails anyway but says less.
    """
    got_version = getattr(fastembed, "__version__", None)
    want_version = contract.PYTHON_FASTEMBED_VERSION
    if got_version != want_version:
        raise PackBuildError(
            f"fastembed {got_version!r} is installed, this sopack build requires "
            f"{want_version!r} — pin the right version before packing. "
            "A different fastembed version can silently use different "
            "pooling, which writes vectors into a different geometric space "
            "than the live collection (this is failure #15).")


def _to_list(vector) -> list[float]:
    tolist = getattr(vector, "tolist", None)
    return tolist() if tolist is not None else list(vector)


def _attr(obj, name, default=None):
    if hasattr(obj, name):
        return getattr(obj, name)
    if isinstance(obj, dict):
        return obj.get(name, default)
    return default


def _load_book(item) -> tuple[object, str | None]:
    """*item* is either a path to a book.json, or a pre-loaded
    ``(book, sha256_or_None)`` pair (how tests hand in a fake Book without a
    real file, and how a caller can reuse an already-parsed Book)."""
    if isinstance(item, tuple) and len(item) == 2:
        return item
    path = Path(item)
    raw = path.read_bytes()
    book_obj = load(path)
    return book_obj, hashlib.sha256(raw).hexdigest()


def _default_pack_id(profile_name: str) -> str:
    stamp = datetime.now(timezone.utc).strftime("%Y-%m-%d")
    return f"{profile_name}-{stamp}-{secrets.token_hex(2)}"


def _default_created_by() -> str:
    return (f"sopack {SOPACK_VERSION} on "
            f"{platform.system()} {platform.release()} {platform.machine()}")


def _load_calibration(calibration) -> tuple[dict, str]:
    """Resolve *calibration* (the ``pack()`` parameter) into ``(fixture_doc,
    fixture_sha256)``.

    ``None`` (the normal case) loads and sha256-verifies the contract's own
    committed fixture via :func:`contract.load_calibration`; the sha is the
    contract's own ``CALIBRATION_SHA256``. A path, dict or list (tests: an
    explicit override) has no committed file to check against, so its sha is
    computed from its own canonical JSON (:func:`contract.calibration_fixture_sha256`)
    — the importer, given that SAME override, computes the identical sha, so
    the two sides still agree on what "this fixture" is."""
    if calibration is None:
        return contract.load_calibration(), contract.CALIBRATION_SHA256
    if isinstance(calibration, (str, Path)):
        try:
            raw = Path(calibration).read_bytes()
        except OSError as exc:
            raise PackBuildError(f"cannot read calibration fixture {calibration}: {exc}") from exc
        try:
            doc = json.loads(raw)
        except json.JSONDecodeError as exc:
            raise PackBuildError(f"{calibration}: not valid JSON: {exc}") from exc
    elif isinstance(calibration, dict):
        doc = calibration
    else:
        doc = {"schema": contract.SCHEMA_CALIBRATION, "entries": list(calibration)}
    return doc, contract.calibration_fixture_sha256(doc)


def _titles_fragment(profile: contract.Profile, book_entries: list[dict]) -> dict | None:
    """Additive fragment matching ``app/data/sop_books.json``'s shape —
    ``{lang: {code: {"titles": [...], author?, year?, corpus?, en_code?}}}`` —
    so it can be merged with exactly the logic in
    ``scripts/merge_corpus_titles.py`` / ``export_book_titles.make_adder``.
    ``bible`` has no title table (see contract.py's profile comparison)."""
    if profile.name != "sop":
        return None
    frag: dict[str, dict[str, dict]] = {}
    for e in book_entries:
        lang, code = e["lang"], e["book_code"]
        entry = frag.setdefault(lang, {}).setdefault(code, {"titles": []})
        title = e.get("title")
        if title and title not in entry["titles"]:
            entry["titles"].append(title)
        if e.get("author") and not entry.get("author"):
            entry["author"] = e["author"]
        if e.get("year") and not entry.get("year"):
            entry["year"] = e["year"]
        if e.get("corpus") and not entry.get("corpus"):
            entry["corpus"] = e["corpus"]
        book_pair = e.get("book_pair") or ""
        if lang != "en" and "/" in book_pair:
            en_code = book_pair.split("/", 1)[1].strip()
            if en_code and not entry.get("en_code"):
                entry["en_code"] = en_code
    return frag


def pack(books, out_path, *, profile: str | None = None,
          pack_id: str | None = None, created_by: str | None = None,
          id_rule: str | None = None, batch_size: int = 1,
          workers: int | None = None, progress=print,
          calibration=None, device: str = "cpu", threads: int = 1) -> dict:
    """Embed every block of *books* and write ``out_path`` as a ``.sopack``.

    ``books`` — paths to ``book.json`` files (or, for tests / reuse,
    ``(book, sha256_or_None)`` pairs already loaded).
    ``calibration`` — ``None`` (the normal case: load and sha256-verify the
    contract's own committed fixture, ``contracts/<id>/calibration.json``), a
    path to an alternate fixture file, or an already-loaded fixture
    doc/entry-list (tests). See :func:`_load_calibration`.
    ``batch_size`` — defaults to **1**, not a larger number: M0 measured
    single-text batches at 2.73 blocks/s on CPU vs 0.99 blocks/s at batch 32
    (large batches are a GPU lever, not a CPU one — SOPACK-1.0-PLAN.md §2/§3.3).
    ``workers`` — ``fastembed`` parallel worker count; ``None``/``0`` (the
    default) means single-process — see the note above the workers-resolution
    line for why this does NOT default to ``os.cpu_count()``. When a caller
    does pass a count, it must come from ``os.cpu_count()``, never a shelled
    out ``nproc`` (R12/#4) — that part of the rule is unaffected.

    Returns the manifest actually written, read back from the finished file —
    so what this function returns is provably what is on disk, not merely
    what it intended to write.
    """
    _preflight()

    if not books:
        raise PackBuildError("no books given")

    loaded = [_load_book(item) for item in books]

    profile_names = {_attr(b, "profile") for b, _ in loaded}
    profile_names.discard(None)
    if profile is not None:
        if profile_names - {profile}:
            raise PackBuildError(
                f"--profile {profile!r} was given but book(s) declare "
                f"{sorted(profile_names - {profile})!r}")
        profile_name = profile
    elif len(profile_names) == 1:
        profile_name = next(iter(profile_names))
    elif not profile_names:
        raise PackBuildError(
            "no book declares a profile and none was given with --profile")
    else:
        raise PackBuildError(
            f"books declare different profiles {sorted(profile_names)!r} — "
            "pack them separately, one profile per .sopack")
    prof = contract.get_profile(profile_name)

    fixture, fixture_sha = _load_calibration(calibration)
    fixture_entries = fixture.get("entries") or []
    if not fixture_entries:
        raise PackBuildError(
            "calibration fixture has no entries — the self-check is not "
            "skippable (R11); see docs/SOPACK-AUTONOMY.md §3.1")

    if id_rule is not None:
        resolved_id_rule = id_rule
    else:
        declared = {_attr(b, "id_rule") for b, _ in loaded}
        declared.discard(None)
        if len(declared) > 1:
            raise PackBuildError(
                f"books declare different id_rules {sorted(declared)!r} in one "
                "pack — a .sopack carries exactly one id_rule; pack them "
                "separately or pass --id-rule to force one")
        resolved_id_rule = next(iter(declared), prof.default_id_rule)
    if resolved_id_rule not in prof.id_rules:
        raise PackBuildError(
            f"id_rule {resolved_id_rule!r} is not valid for profile "
            f"{prof.name!r} (allowed: {', '.join(prof.id_rules)})")

    pack_id = pack_id or _default_pack_id(prof.name)
    created_by = created_by or _default_created_by()
    # `workers` is deliberately NOT defaulted to os.cpu_count() here, even
    # though R12/#4 says a worker count must come from os.cpu_count() and
    # never `nproc` — that rule is about *how* a count is obtained when one is
    # wanted, not a mandate to always want one. fastembed's `parallel=N`
    # forks N worker subprocesses that EACH load their own copy of the ~2.2 GB
    # model; the parent process here has already loaded it once (this
    # function needs a single model instance for both books and canaries —
    # see the module docstring / R11), and forking after onnxruntime's
    # internal thread pool is already up is a known deadlock class. Measured
    # on this machine: `workers=os.cpu_count()` (4) against a real ~2500-block
    # book hung for 10 minutes and then failed with `_queue.Empty` — this
    # container's `MemAvailable` was ~1.9 GB, nowhere near 4x2.2 GB. Only use
    # multiple processes when a caller explicitly opts in with `--workers`,
    # on a machine known to have the RAM for it (the reference one-off's own
    # comment: "try (cores - 2) on a Mac").
    workers = workers or 0

    progress(f"loading {contract.EMBEDDING['model']} …")
    model = TextEmbedding(model_name=contract.EMBEDDING["model"])
    prefix = contract.EMBEDDING["passage_prefix"]
    embed_kwargs = {"batch_size": batch_size}
    if workers:
        embed_kwargs["parallel"] = workers

    # Calibration self-check (SOPACK-AUTONOMY.md §3.1, R11) — BEFORE any book
    # is embedded, with the same model instance and settings the books will
    # use. A broken environment (wrong pooling, a bad ORT build, …) then
    # fails in seconds instead of after however long the books take.
    progress(f"calibration self-check: embedding {len(fixture_entries)} "
             f"fixture entr{'y' if len(fixture_entries) == 1 else 'ies'} …")
    fixture_texts = [prefix + e["text"] for e in fixture_entries]
    fixture_vectors = [_to_list(v) for v in model.embed(fixture_texts, **embed_kwargs)]
    for v in fixture_vectors:
        if len(v) != contract.VECTOR_SIZE:
            raise PackBuildError(
                f"calibration embedding produced dimension {len(v)}, contract "
                f"requires {contract.VECTOR_SIZE}")
    cosines = [contract.cosine(fv, e["vector"]) for fv, e in zip(fixture_vectors, fixture_entries)]
    min_cosine = min(cosines)
    mean_cosine = sum(cosines) / len(cosines)
    if min_cosine < contract.PACK_MIN_COSINE:
        raise CalibrationFailed(
            f"calibration self-check FAILED: min cosine {min_cosine:.8f} < "
            f"{contract.PACK_MIN_COSINE} (mean {mean_cosine:.8f}, n={len(cosines)}) — "
            "this machine's embeddings do not reproduce the committed fixture's; "
            "refusing to embed any book. This is exactly the failure the "
            "calibration gate exists to catch (a pooling/library/model change "
            "silently moving vectors into a different geometric space) — see "
            "docs/SOPACK-AUTONOMY.md §3.1.")
    progress(f"  calibration ok: min cosine {min_cosine:.8f}, mean {mean_cosine:.8f} "
             f"(threshold {contract.PACK_MIN_COSINE})")
    self_check = {"n": len(cosines), "min_cosine": min_cosine, "mean_cosine": mean_cosine,
                  "threshold": contract.PACK_MIN_COSINE}

    out_path = Path(out_path)
    book_entries: list[dict] = []
    seen_dim: int | None = None

    with PackWriter(out_path, profile=prof.name, pack_id=pack_id,
                     created_by=created_by, id_rule=resolved_id_rule,
                     dim=contract.VECTOR_SIZE, device=device, threads=threads,
                     batch_tokens=batch_size) as writer:
        for book_obj, book_sha256 in loaded:
            blocks = _attr(book_obj, "blocks") or []
            blocks = list(blocks)
            if not blocks:
                progress("  (skipping book with no blocks)")
                continue

            payloads = [to_payload(book_obj, block) for block in blocks]
            uids = [uid(book_obj, block) for block in blocks]
            texts = [prefix + p[prof.text_field] for p in payloads]

            n_done = 0
            first_id = None
            book_label = payloads[0].get(prof.identity, "?")
            for start in range(0, len(texts), batch_size):
                chunk_texts = texts[start:start + batch_size]
                vectors = model.embed(chunk_texts, **embed_kwargs)
                for offset, vec in enumerate(vectors):
                    i = start + offset
                    vec = _to_list(vec)
                    if seen_dim is None:
                        seen_dim = len(vec)
                        if seen_dim != contract.VECTOR_SIZE:
                            raise PackBuildError(
                                f"embedding produced dimension {seen_dim}, "
                                f"contract requires {contract.VECTOR_SIZE}")
                    pid = writer.add(uids[i], payloads[i], vec)
                    if first_id is None:
                        first_id = pid
                    n_done += 1
                progress(f"  {book_label}: {n_done}/{len(texts)} blocks embedded "
                         f"| {writer.count} total")

            meta = payloads[0]
            book_entries.append({
                "book_code": meta.get(prof.identity, book_label),
                "lang": meta.get("lang"),
                "points": n_done,
                "first_id": first_id,
                "title": meta.get("title"),
                "author": meta.get("author"),
                "year": meta.get("year"),
                "corpus": meta.get("corpus"),
                "slug": meta.get("slug"),
                "book_pair": meta.get("book_pair"),
                "id_rule": resolved_id_rule,
                "book_sha256": book_sha256,
            })

        writer.set_books(book_entries)
        writer.set_probe(fixture_entries, fixture_vectors, self_check=self_check,
                         fixture_sha256=fixture_sha)
        titles = _titles_fragment(prof, book_entries)
        if titles is not None:
            writer.set_titles(titles)

    from .format import PackReader
    with PackReader(out_path) as reader:
        manifest = reader.manifest
    progress(f"wrote {out_path} — {manifest['counts']['points']:,} points, "
             f"{manifest['counts']['books']} books")
    return manifest
