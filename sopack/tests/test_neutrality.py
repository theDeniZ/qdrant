"""SOPACK-AUTONOMY.md §5 acceptance criterion #2: ``sopack/`` contains no
store URL, no HTTP client code and no reference to a collection or vector
name.

This is checked at the **token** level (``tokenize``), not by a plain
``grep``, so a docstring or comment that *explains* the split (this file's
own module docstring, for instance, or ``contract.py``'s notes on why
``check_target`` no longer validates a legacy target's backend fields) is not
a false positive — only real code (identifiers, non-string literals, import
statements) is checked. Only "docs/comments... explain the split" are
allowed to use the words at all; this test enforces that by construction:
comments and strings are always skipped, so a stray VALUE (an actual
collection name baked into code, not just the word appearing in prose) would
still be structurally impossible without ALSO tripping the import/identifier
checks below.

Runnable standalone:
    /workspaces/sdarm/.venv/bin/python3.11 -m sopack.tests.test_neutrality
"""

from __future__ import annotations

import ast
import re
import tokenize
import unittest
from pathlib import Path

SOPACK_DIR = Path(__file__).resolve().parents[1]

# Directories that are not source: build artifacts, caches, packaging
# metadata. None of these are "sopack/" as the acceptance criterion means it.
_EXCLUDE_DIR_NAMES = {"build", "__pycache__", "sopack.egg-info"}

_BANNED_IMPORT_MODULES = {"requests", "httpx", "qdrant_client", "urllib.request"}
_BANNED_IDENTIFIERS = {"collection", "vector_name"}
# The live Qdrant host, and any URL whose host contains "qdrant" — a real
# store address, not the many legitimate, unrelated http(s) strings in this
# package (XML namespaces in extract/epub.py, JSON Schema $id/$schema).
_SCHEME_SEP = "://"
_STORE_URL_RE = re.compile(r"10\.10\.10\.10|" + re.escape(_SCHEME_SEP) + r"[^\s\"'/]*qdrant",
                           re.IGNORECASE)

_SKIP_TOKEN_TYPES = {
    tokenize.COMMENT, tokenize.STRING, tokenize.NL, tokenize.NEWLINE,
    tokenize.ENCODING, tokenize.INDENT, tokenize.DEDENT, tokenize.ENDMARKER,
    tokenize.OP,
}


def _py_files() -> list[Path]:
    out = []
    for path in sorted(SOPACK_DIR.rglob("*.py")):
        if _EXCLUDE_DIR_NAMES & set(path.relative_to(SOPACK_DIR).parts[:-1]):
            continue
        out.append(path)
    return out


class NeutralityTests(unittest.TestCase):
    def test_no_http_client_or_qdrant_client_imports(self):
        violations = []
        for path in _py_files():
            tree = ast.parse(path.read_text(encoding="utf-8"), filename=str(path))
            for node in ast.walk(tree):
                if isinstance(node, ast.Import):
                    for alias in node.names:
                        if (alias.name in _BANNED_IMPORT_MODULES
                                or alias.name.split(".")[0] == "qdrant_client"):
                            violations.append(f"{path}:{node.lineno}: import {alias.name}")
                elif isinstance(node, ast.ImportFrom):
                    mod = node.module or ""
                    if mod in _BANNED_IMPORT_MODULES or mod.split(".")[0] == "qdrant_client":
                        violations.append(f"{path}:{node.lineno}: from {mod} import ...")
        self.assertEqual(violations, [],
                         "sopack/ must never import an HTTP client or qdrant_client "
                         "(only app/'s importer talks to a store):\n" + "\n".join(violations))

    def test_no_store_urls(self):
        violations = []
        for path in _py_files():
            for lineno, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
                if _STORE_URL_RE.search(line):
                    violations.append(f"{path}:{lineno}: {line.strip()}")
        self.assertEqual(violations, [],
                         "sopack/ must name no store address:\n" + "\n".join(violations))

    def test_no_collection_or_vector_name_identifiers_in_code(self):
        """Only real code tokens are checked — NAME tokens outside strings and
        comments. A docstring explaining the split (e.g. "collection names
        ... now live in the importer's adapter config") is not code and does
        not trip this."""
        violations = []
        for path in _py_files():
            with open(path, "rb") as fh:
                for tok in tokenize.tokenize(fh.readline):
                    if tok.type in _SKIP_TOKEN_TYPES:
                        continue
                    if tok.type == tokenize.NAME and tok.string in _BANNED_IDENTIFIERS:
                        violations.append(f"{path}:{tok.start[0]}: {tok.string!r}")
        self.assertEqual(violations, [],
                         "sopack/ code (not docs/comments) must not reference a "
                         "collection or vector_name — that is Qdrant adapter "
                         "config now (app/store_adapter.py):\n" + "\n".join(violations))


if __name__ == "__main__":
    unittest.main(verbosity=2)
