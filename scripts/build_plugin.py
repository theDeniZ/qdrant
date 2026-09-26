#!/usr/bin/env python3
"""Assemble the ``bible-sop`` plugin.

* ``references/*.md`` (canonical, edit these) -> ``plugin/skills/<skill>/references/``
  for every skill, so each skill is self-contained when installed on its own.
* Validates every ``SKILL.md`` frontmatter (``name`` = directory, description
  present and <= 1024 chars) and that every linked ``references/…`` file exists.
* Writes ``dist/bible-sop.zip`` (the plugin root) and ``dist/skills/<skill>.zip``
  (one skill folder each) for upload to claude.ai.

Usage:
    python3 scripts/build_plugin.py          # sync + validate + zip
    python3 scripts/build_plugin.py --check  # exit 1 if stale or invalid, write nothing
"""

from __future__ import annotations

import argparse
import re
import sys
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PLUGIN = ROOT / "plugin"
REFS = ROOT / "references"
DIST = ROOT / "dist" / "bible-sop.zip"

_FRONTMATTER = re.compile(r"\A---\n(.*?)\n---\n", re.S)
_LINK = re.compile(r"\]\((references/[^)#]+)\)")


def skills() -> list[Path]:
    return sorted(p for p in (PLUGIN / "skills").iterdir() if (p / "SKILL.md").is_file())


def expected_files() -> dict[Path, bytes]:
    return {skill / "references" / ref.name: ref.read_bytes()
            for skill in skills() for ref in sorted(REFS.glob("*.md"))}


def validate() -> list[str]:
    errors = []
    for skill in skills():
        text = (skill / "SKILL.md").read_text(encoding="utf-8")
        m = _FRONTMATTER.match(text)
        if not m:
            errors.append(f"{skill.name}: SKILL.md has no frontmatter")
            continue
        meta = dict(line.split(":", 1) for line in m.group(1).splitlines() if ":" in line)
        if meta.get("name", "").strip() != skill.name:
            errors.append(f"{skill.name}: frontmatter name {meta.get('name', '').strip()!r} != directory")
        desc = meta.get("description", "").strip()
        if not desc or len(desc) > 1024:
            errors.append(f"{skill.name}: description missing or longer than 1024 chars ({len(desc)})")
        for link in _LINK.findall(text):
            if not (REFS / Path(link).name).is_file():
                errors.append(f"{skill.name}: broken link {link}")
    return errors


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--check", action="store_true", help="report stale or invalid files, write nothing")
    args = ap.parse_args()

    errors = validate()
    expected = expected_files()
    stale = [p for p, data in expected.items() if not p.exists() or p.read_bytes() != data]
    extra = [p for skill in skills() for p in (skill / "references").glob("*")
             if p not in expected] if not args.check else []

    for e in errors:
        print(f"error: {e}")
    if args.check:
        for p in stale:
            print(f"stale: {p.relative_to(ROOT)}")
        return 1 if (stale or errors) else 0
    if errors:
        return 1

    for p, data in expected.items():
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_bytes(data)
    for p in extra:
        p.unlink()

    DIST.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(DIST, "w", zipfile.ZIP_DEFLATED) as zf:
        for f in sorted(PLUGIN.rglob("*")):
            if f.is_file():
                zf.write(f, f.relative_to(PLUGIN))
    # One zip per skill (folder at the zip root) for per-skill upload; drop zips
    # of skills that no longer exist.
    for old in (DIST.parent / "skills").glob("*.zip"):
        old.unlink()
    for skill in skills():
        target = DIST.parent / "skills" / f"{skill.name}.zip"
        target.parent.mkdir(parents=True, exist_ok=True)
        with zipfile.ZipFile(target, "w", zipfile.ZIP_DEFLATED) as zf:
            for f in sorted(skill.rglob("*")):
                if f.is_file():
                    zf.write(f, Path(skill.name) / f.relative_to(skill))
    print(f"synced {len(stale)} file(s), removed {len(extra)}; wrote {DIST.relative_to(ROOT)} "
          f"and dist/skills/<skill>.zip")
    return 0


if __name__ == "__main__":
    sys.exit(main())
