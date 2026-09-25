#!/usr/bin/env python3
"""Generate golden `{rule, fields, uid, id}` cases from the Python contract
(``sopack.contract``) for the Rust `sopack-contract` crate to reproduce.

This is the M2 "point ids for every id rule" leg of the conformance suite
(SOPACK-1.0-PLAN.md §4): the Rust and Python sides each implement the same
id-rule templates (`[ids.rules]` in contract.toml — `sop/plain`, `sop/seq`,
`bible/v1`) independently, and this script is what proves they agree,
instead of trusting two hand-written implementations to match by
inspection.

It deliberately calls ``sopack.contract.uid_for``/``point_id`` — the
single-source-of-truth functions every other Python module in this pipeline
already goes through — rather than reimplementing the id rules itself, so
the goldens always reflect whatever the Python contract module currently
does (hardcoded `ID_RULES` functions today; template-driven once the
concurrent contract.toml migration lands — the *templates* are identical
either way, since they are transcribed 1:1 into
`contracts/e5-large-v1/contract.toml`'s `[ids.rules]`, so this script needs
no change across that migration).

Cases cover: every rule, `seq` explicitly 0, a non-zero `seq`, and Unicode
in `book_code`/`para_key`/`bible` text fields — the exact soft spots
SOPACK-2-FORMAT.md's id rules can drift on (non-ASCII `f"{...}"`
formatting). `seq` is never *absent* here on purpose: `contract._fmt_uid`
is `template.format(**fields)` with no defaulting of any field, including
`seq` — a template needing a field that is not in `fields` raises. The
"seq defaults to 0 when the uid has no `#`" behavior lives one layer up, in
`sopack.format._fields`/`sopack-format::idfields::fields_from`, which is
covered by that crate's own unit tests, not this contract-level golden
set.

Run:
    PYTHONPATH=/workspaces/sdarm/qdrant /workspaces/sdarm/.venv/bin/python3.11 \\
        qdrant/sopack-rs/conformance/ids/make_id_goldens.py

Writes ``golden.json`` next to this script. Regenerate whenever
`sopack/contract.py`'s id rules change (which should be never without a new
contract version — see contract.toml's own warning about that).
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[3]))  # .../qdrant
from sopack import contract  # noqa: E402

CASES: list[tuple[str, dict]] = [
    # sop/plain — no seq concept at all.
    ("sop/plain", {"lang": "en", "book_code": "WDYS", "para_key": "1.1"}),
    ("sop/plain", {"lang": "de", "book_code": "GC", "para_key": "12.7"}),
    # sop/seq — seq explicitly 0.
    ("sop/seq", {"lang": "en", "book_code": "WDYS", "para_key": "1.1", "seq": 0}),
    # sop/seq — non-zero seq.
    ("sop/seq", {"lang": "en", "book_code": "WDYS", "para_key": "1.1", "seq": 3}),
    ("sop/seq", {"lang": "de", "book_code": "GC", "para_key": "12.7", "seq": 12}),
    # Unicode in book_code / para_key (German umlauts, a section-mark, CJK).
    ("sop/plain", {"lang": "de", "book_code": "Ünïcödé-Büch", "para_key": "Kapítel§1"}),
    ("sop/seq", {"lang": "ja", "book_code": "ABC", "para_key": "1.1", "seq": 0}),
    ("sop/seq", {"lang": "ko", "book_code": "테스트", "para_key": "1.1", "seq": 5}),
    ("sop/seq", {"lang": "ru", "book_code": "ИСПЫТАНИЯ", "para_key": "3.14", "seq": 1}),
    # bible/v1.
    ("bible/v1", {"bible": "kjv", "osis": "Gen.1.1"}),
    ("bible/v1", {"bible": "luther1912", "osis": "Ps.51.1"}),
    ("bible/v1", {"bible": "elberfelder1905", "osis": "Röm.8.28"}),
    ("bible/v1", {"bible": "japkougo", "osis": "John.3.16"}),
]


def main() -> None:
    golden = []
    for rule, fields in CASES:
        uid = contract.uid_for(rule, fields)
        pid = contract.point_id(rule, fields)
        golden.append({"rule": rule, "fields": fields, "uid": uid, "id": pid})

    out = Path(__file__).with_name("golden.json")
    out.write_text(json.dumps(golden, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {len(golden)} cases to {out}")


if __name__ == "__main__":
    main()
