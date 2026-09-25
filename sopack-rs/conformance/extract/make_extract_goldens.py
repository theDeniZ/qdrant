#!/usr/bin/env python3
"""Builds `sopack-rs/conformance/extract/`'s fixture sources and runs the
Python `sopack.extract` reference against them (and against a small set of
real pioneer EPUBs, if present) to produce the `*.book.json` goldens the
Rust `sopack-extract` conformance test (`crates/sopack-extract/tests/
conformance_extract.rs`) must reproduce byte-for-byte.

Run from `qdrant/`:

    PYTHONPATH=. .venv/bin/python3.11 sopack-rs/conformance/extract/make_extract_goldens.py

(the repo's venv is `/workspaces/sdarm/.venv`; invoke it directly if `.venv/`
is not on PATH from this directory).

Deterministic: the fixture sources this script writes have fixed content
(no timestamps, no randomness), so re-running it reproduces byte-identical
fixtures and goldens. The `real_books` entries in `manifest.json` reference
existing files under `qdrant/pd-books/converted/` by path — this script
never copies or modifies them, and skips (with a printed note) any that are
absent, per the task's "do not copy huge files into conformance" rule.
"""

from __future__ import annotations

import json
import sys
import zipfile
from pathlib import Path

QDRANT_ROOT = Path(__file__).resolve().parents[3]  # .../qdrant
CONFORMANCE_DIR = Path(__file__).resolve().parent  # .../qdrant/sopack-rs/conformance/extract
FIXTURES_DIR = CONFORMANCE_DIR / "fixtures"
GOLDENS_DIR = CONFORMANCE_DIR / "goldens"

sys.path.insert(0, str(QDRANT_ROOT))

from sopack import book as book_mod  # noqa: E402
from sopack.extract import extract  # noqa: E402


# ── fixture sources ──────────────────────────────────────────────────────

_CONTAINER_XML = """<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"""


def _opf(title: str, creator: str, date: str, items: tuple[str, ...]) -> str:
    manifest_items = "\n".join(
        f'<item id="c{i}" href="{h}" media-type="application/xhtml+xml"/>'
        for i, h in enumerate(items))
    spine_items = "\n".join(f'<itemref idref="c{i}"/>' for i in range(len(items)))
    return f"""<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>{title}</dc:title>
    <dc:creator>{creator}</dc:creator>
    <dc:date>{date}</dc:date>
    <dc:rights>Public domain</dc:rights>
  </metadata>
  <manifest>
    {manifest_items}
  </manifest>
  <spine>
    {spine_items}
  </spine>
</package>"""


def write_small_epub(path: Path) -> None:
    """Two ordinary paragraphs, no inline citation scheme, no page markers —
    exercises OPF `dc:*` metadata extraction (namespace resolution) and the
    chapter.ordinal page/para fallback."""
    chapters = [
        "<p>This is the first paragraph of real prose in the conformance fixture.</p>"
        "<p>This is the second paragraph, also real prose, quite readable indeed.</p>",
    ]
    names = [f"chap{i}.xhtml" for i in range(len(chapters))]
    with zipfile.ZipFile(path, "w") as zf:
        zf.writestr("mimetype", "application/epub+zip")
        zf.writestr("META-INF/container.xml", _CONTAINER_XML)
        zf.writestr("OEBPS/content.opf", _opf("Test Work", "J. Q. Author", "1877", tuple(names)))
        for name, body in zip(names, chapters):
            zf.writestr(f"OEBPS/{name}", f'<?xml version="1.0"?><html><body>{body}</body></html>')


def write_small_markdown(path: Path) -> None:
    path.write_text(
        "# Chapter One\n\n"
        "This is the first paragraph of chapter one, with plenty of words to pass every gate.\n\n"
        "This is the second paragraph of chapter one, likewise readable nineteenth century prose.\n\n"
        "# Chapter Two\n\n"
        "This is the first paragraph of chapter two, again with enough words in it to survive.\n",
        encoding="utf-8",
    )


def write_small_text(path: Path) -> None:
    path.write_text(
        "This is the first paragraph of plain text with enough words in it to pass every gate here.\n\n"
        "This is the second paragraph of plain text, also long enough to survive the quality gate.\n",
        encoding="utf-8",
    )


def write_small_sop_json(path: Path) -> None:
    doc = {
        "meta": {"en_code": "SMALL", "en_title": "A Small Fixture Book", "publisher": "Conformance"},
        "en": {
            "0.1": {"text": "This is the first paragraph of the English conformance fixture.",
                    "chapter": "Preface"},
            "0.2": {"text": "This is the second paragraph, also part of the conformance preface.",
                    "chapter": "Preface"},
        },
    }
    path.write_text(json.dumps(doc), encoding="utf-8")


def write_small_sop_json_de(path: Path) -> None:
    """German file with `en_reverse` alignment — exercises the alignment
    passthrough and `to_payload`'s `aligned` lookup."""
    doc = {
        "meta": {"de_code": "SMALLDE", "en_code": "SMALL", "de_title": "Ein kleines Testbuch",
                 "en_title": "A Small Fixture Book", "publisher": "Conformance", "year": "1973"},
        "de": {
            "5.1": {"text": "Dies ist der erste Absatz des deutschen Testbuchs fuer die Konformitaet.",
                    "en_ref": "0.1", "chapter": "Kapitel 1"},
            "5.2": {"text": "Dies ist der zweite Absatz, ebenfalls Teil des deutschen Testbuchs.",
                    "en_ref": "0.2", "chapter": "Kapitel 1"},
        },
        "en_reverse": {"0.1": ["5.1"], "0.2": ["5.2"]},
    }
    path.write_text(json.dumps(doc), encoding="utf-8")


_FIXTURE_WRITERS = {
    "sopack-rs/conformance/extract/fixtures/small.epub": write_small_epub,
    "sopack-rs/conformance/extract/fixtures/small.md": write_small_markdown,
    "sopack-rs/conformance/extract/fixtures/small.txt": write_small_text,
    "sopack-rs/conformance/extract/fixtures/en/SMALL.json": write_small_sop_json,
    "sopack-rs/conformance/extract/fixtures/de/SMALLDE.json": write_small_sop_json_de,
}


def main() -> int:
    manifest = json.loads((CONFORMANCE_DIR / "manifest.json").read_text(encoding="utf-8"))
    GOLDENS_DIR.mkdir(parents=True, exist_ok=True)

    for entry in manifest["fixtures"]:
        rel = entry["source"]
        writer = _FIXTURE_WRITERS.get(rel)
        if writer is None:
            print(f"no fixture writer registered for {rel!r}", file=sys.stderr)
            return 2
        full_path = QDRANT_ROOT / rel
        full_path.parent.mkdir(parents=True, exist_ok=True)
        writer(full_path)

    entries = manifest["fixtures"] + manifest["real_books"]
    made, skipped = 0, 0
    for entry in entries:
        rel = entry["source"]
        full_path = QDRANT_ROOT / rel
        if not full_path.exists():
            print(f"skip {entry['name']}: {rel} not found (real-book fixture not present locally)")
            skipped += 1
            continue
        # `extract()` receives the RELATIVE (to qdrant/) path string, so
        # book.json's source.file matches whatever the Rust conformance
        # test passes on its side, byte for byte.
        book = extract(rel, entry["kind"], **entry["options"])
        out = GOLDENS_DIR / f"{entry['name']}.book.json"
        book_mod.dump(book, out)
        print(f"wrote {out.relative_to(QDRANT_ROOT)}")
        made += 1

    print(f"{made} golden(s) written, {skipped} skipped")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
