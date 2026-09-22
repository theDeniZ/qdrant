"""Unit tests for sopack.verify.

Runnable standalone:
    /workspaces/sdarm/.venv/bin/python3.11 -m sopack.tests.test_verify
"""

from __future__ import annotations

import json
import sys
import tempfile
import unittest
import zipfile
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))

from sopack.tests import _helpers  # noqa: E402

# See test_pack.py for why this is bracketed rather than a bare
# sys.modules assignment: sopack.pack only ever does `from .book import ...`
# once (module import time), so the fake only needs to be in place for that
# one import; standalone (`-m unittest sopack.tests.test_verify`) this is
# also the *first* import of sopack.pack in the process, so the bracket is
# required here too, not just in test_pack.py.
with _helpers.patched_book_module() as _book_mod:
    from sopack import pack as pack_mod  # noqa: E402

from sopack import contract  # noqa: E402
from sopack import verify as verify_mod  # noqa: E402


def _canaries(n=2):
    return [{"id": f"canary-{i}", "collection": "sop", "text": f"canary text {i}"}
            for i in range(n)]


def _rewrite_zip_entry(path: Path, name: str, data: bytes) -> None:
    with zipfile.ZipFile(path) as zf:
        entries = {n: zf.read(n) for n in zf.namelist()}
    entries[name] = data
    with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as zf:
        for n, blob in entries.items():
            zf.writestr(n, blob)


def _rewrite_manifest(path: Path, mutate) -> None:
    with zipfile.ZipFile(path) as zf:
        manifest = json.loads(zf.read("manifest.json"))
    mutate(manifest)
    _rewrite_zip_entry(path, "manifest.json", json.dumps(manifest).encode("utf-8"))


class VerifyTests(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.out = Path(self._tmp.name) / "test.sopack"

    def _build_pack(self):
        book = _helpers.make_book(_book_mod, lang="en", book_code="X", n_blocks=3,
                                   title="Book X")
        with mock.patch.object(
                pack_mod, "TextEmbedding",
                lambda model_name=None, **kw: _helpers.FakeTextEmbedding(
                    model_name, dim=contract.VECTOR_SIZE)):
            pack_mod.pack([(book, None)], self.out, _canaries(2),
                          workers=1, progress=lambda *_: None)

    def test_clean_pack_has_no_errors(self):
        self._build_pack()
        errors = verify_mod.verify(self.out)
        self.assertEqual(errors, [])

    def test_detects_manifest_book_count_mismatch(self):
        self._build_pack()

        def mutate(manifest):
            manifest["books"][0]["points"] = 999

        _rewrite_manifest(self.out, mutate)
        errors = verify_mod.verify(self.out)
        self.assertTrue(errors, "expected a mismatch to be reported")
        self.assertTrue(any("999" in e for e in errors), errors)

    def test_detects_corrupted_vectors(self):
        self._build_pack()
        with zipfile.ZipFile(self.out) as zf:
            original = zf.read("vectors.f32")
        _rewrite_zip_entry(self.out, "vectors.f32", original[:-8])  # drop one float

        errors = verify_mod.verify(self.out)
        self.assertTrue(errors, "expected corruption to be reported")
        self.assertTrue(any("sha256" in e or "vectors.f32" in e for e in errors), errors)

    def test_missing_pack_reports_error_not_exception(self):
        errors = verify_mod.verify(Path(self._tmp.name) / "does-not-exist.sopack")
        self.assertTrue(errors)


if __name__ == "__main__":
    unittest.main()
