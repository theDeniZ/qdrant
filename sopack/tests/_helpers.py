"""Test doubles shared by test_pack.py / test_verify.py.

Not a pipeline deliverable — just avoids duplicating the fake ``sopack.book``
seam and the fake embedder across both test files.

``sopack.book`` is owned by another agent and does not exist on disk yet (and
this package must never create it — see the task's hard rules). So instead of
importing the real thing, these tests install a minimal fake module into
``sys.modules['sopack.book']`` *before* ``sopack.pack`` is ever imported —
``from .book import ...`` inside ``sopack/pack.py`` resolves against
``sys.modules`` first, so no file needs to exist on disk for this to work.
Real fastembed vectors are never used either: ``FakeTextEmbedding`` produces
small deterministic vectors from a seeded PRNG, so tests run in milliseconds
and never touch the 2 GB model.
"""

from __future__ import annotations

import contextlib
import random
import sys
import types


def _build_fake_book_module():
    mod = types.ModuleType("sopack.book")

    class BookError(Exception):
        pass

    class Block:
        def __init__(self, para_key, text, page=1, para=1, seq=0):
            self.para_key = para_key
            self.text = text
            self.page = page
            self.para = para
            self.seq = seq

    class Book:
        def __init__(self, *, lang, book_code, blocks, title=None, author=None,
                     year=None, corpus=None, slug=None, book_pair=None,
                     profile="sop", id_rule="sop/seq"):
            self.lang = lang
            self.book_code = book_code
            self.title = title
            self.author = author
            self.year = year
            self.corpus = corpus
            self.slug = slug
            self.book_pair = book_pair or book_code
            self.blocks = blocks
            self.profile = profile
            self.id_rule = id_rule
            self.book = {
                "lang": lang, "book_code": book_code, "title": title,
                "author": author, "year": year, "corpus": corpus,
                "slug": slug, "book_pair": self.book_pair,
            }
            self.stats = {"blocks_in": len(blocks), "blocks_out": len(blocks)}

    def load(path):
        raise BookError(f"fake sopack.book.load() should not be called ({path})")

    def to_payload(book, block):
        payload = {
            "lang": book.lang,
            "book_code": book.book_code,
            "book_pair": book.book_pair,
            "page": block.page,
            "para": block.para,
            "para_key": block.para_key,
            "raw_text": block.text,
            "aligned": None,
        }
        for key in ("title", "author", "year", "corpus", "slug"):
            value = getattr(book, key)
            if value is not None:
                payload[key] = value
        return payload

    def uid(book, block):
        return f"{book.lang}:{book.book_code}:{block.para_key}#{block.seq}"

    mod.BookError = BookError
    mod.Block = Block
    mod.Book = Book
    mod.load = load
    mod.to_payload = to_payload
    mod.uid = uid
    return mod


@contextlib.contextmanager
def patched_book_module():
    """Swaps ``sys.modules['sopack.book']`` for the fake for the duration of
    the ``with`` block, then puts back whatever was there before (the real
    module, if some other test module already imported it — or nothing).

    This module's only real client is ``sopack.pack``'s one-time, module-level
    ``from .book import ...`` — so callers use this to bracket their *first*
    ``from sopack import pack`` and nothing else. It must not leak: other test
    modules in this package (``test_book.py`` / ``test_extract.py``) do
    ``from sopack.book import to_payload`` **inside their test methods**, at
    run time, not at import time — if the fake were left installed in
    ``sys.modules`` after this context manager exits, those later, dynamic
    imports would silently pick up this fake instead of the real module."""
    fake = _build_fake_book_module()
    previous = sys.modules.get("sopack.book")
    sys.modules["sopack.book"] = fake
    try:
        yield fake
    finally:
        if previous is not None:
            sys.modules["sopack.book"] = previous
        else:
            sys.modules.pop("sopack.book", None)


def fake_calibration(texts: list[str], *, dim: int = 1024, profile: str = "sop") -> dict:
    """A calibration fixture doc whose stored vectors are exactly what
    :class:`FakeTextEmbedding` will (re)produce for ``passage_prefix + text``
    — so a test's calibration self-check (``sopack.pack.pack``'s first step)
    passes at cosine 1.0 without touching the real model. Import
    ``contract.EMBEDDING["passage_prefix"]`` lazily to avoid a module-level
    dependency loop with ``sopack.contract``."""
    from sopack import contract
    prefix = contract.EMBEDDING["passage_prefix"]
    embedder = FakeTextEmbedding(dim=dim)
    entries = []
    for i, text in enumerate(texts):
        vector = next(iter(embedder.embed([prefix + text])))
        entries.append({"id": f"fixture-{i}", "profile": profile, "uid": None,
                        "lang": "en", "note": f"test fixture {i}", "text": text,
                        "vector": vector})
    return {"schema": "sopack.calibration/1", "contract": "e5-large-v1",
            "entries": entries}


class FakeTextEmbedding:
    """Stands in for ``fastembed.TextEmbedding``: deterministic, instant,
    seeded off each text's hash so the same text always gets the same
    vector within one process."""

    def __init__(self, model_name=None, dim=1024, **_kw):
        self.model_name = model_name
        self.dim = dim

    def embed(self, texts, batch_size=None, **_kw):
        for text in texts:
            rnd = random.Random(f"sopack-fake-embed:{text}")
            yield [rnd.random() for _ in range(self.dim)]


def make_book(book_helpers_mod, *, lang, book_code, n_blocks=3, **kw):
    Block = book_helpers_mod.Block
    Book = book_helpers_mod.Book
    blocks = [Block(f"{i + 1}.1", f"{book_code} block {i + 1} text.", page=i + 1)
              for i in range(n_blocks)]
    return Book(lang=lang, book_code=book_code, blocks=blocks, **kw)
