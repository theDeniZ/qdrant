#!/usr/bin/env python3
"""Export SoP book code → title tables for ``sop_list_books``.

Reads the generator's SoP data directory (``book_map.json`` plus the
``en/<CODE>.json`` meta blocks) and writes ``app/data/sop_books.json``::

    {"en": {"SC": {"titles": ["Steps to Christ"]}, ...},
     "de": {"BW": {"titles": ["Der bessere Weg ..."], "en_code": "SC"}, ...},
     "ja": {...}, "ko": {...}}

Languages not listed (es, ru, …) use English book codes, so the server falls
back to the ``en`` titles for them. Re-run after the corpus gains books:

    python scripts/export_book_titles.py /path/to/generator/data/sop
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

OUT = Path(__file__).resolve().parent.parent / "app" / "data" / "sop_books.json"


def main() -> int:
    if len(sys.argv) != 2:
        print(__doc__)
        return 2
    sop = Path(sys.argv[1])
    book_map = json.loads((sop / "book_map.json").read_text(encoding="utf-8"))
    out: dict[str, dict[str, dict]] = {}

    def add(lang: str, code: str, title: str, en_code: str | None = None) -> None:
        entry = out.setdefault(lang, {}).setdefault(code, {"titles": []})
        if title and title not in entry["titles"]:
            entry["titles"].append(title)
        if en_code and lang != "en" and en_code not in ("N/A", "n/a"):
            entry["en_code"] = en_code

    for path in sorted((sop / "en").glob("*.json")):
        meta = json.loads(path.read_text(encoding="utf-8")).get("meta", {})
        add("en", meta.get("en_code", path.stem), meta.get("en_title", ""))
    for lang, titles in book_map.items():
        for title, v in titles.items():
            code = v.get(f"{lang}_code")
            if code:
                add(lang, code, title, v.get("en_code"))
            if v.get("en_code") and v.get("en_title"):
                add("en", v["en_code"], v["en_title"])
    for books in out.values():
        for entry in books.values():
            entry["titles"].sort(key=len)

    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(json.dumps(out, ensure_ascii=False, indent=1, sort_keys=True) + "\n", encoding="utf-8")
    print(f"wrote {OUT} ({', '.join(f'{k}: {len(v)}' for k, v in sorted(out.items()))})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
