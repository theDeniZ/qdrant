#!/usr/bin/env python3
"""Unit tests for sopack.book — the fixed-seam Book API.

Run:  /workspaces/sdarm/.venv/bin/python3.11 sopack/tests/test_book.py
"""
from __future__ import annotations

import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))  # qdrant/

from sopack import contract
from sopack.book import Block, Book, BookError, dump, load, to_payload, uid, validate


def _plain_sop_book(**overrides) -> Book:
    meta = {
        "book_code": "ABC",
        "lang": "en",
        "book_pair": "ABC",
        "title": "A Book",
        "author": None,
        "year": None,
        "slug": "a-book",
        "corpus": None,
        "page_kind": None,
    }
    meta.update(overrides)
    blocks = [
        Block(para_key="1.1", page=1, para=1, seq=0, chunks=1, text="Hello world today.", words=3),
        Block(para_key="1.2", page=1, para=2, seq=0, chunks=1, text="A second paragraph here.", words=4),
    ]
    return Book(
        schema=contract.SCHEMA_BOOK,
        profile="sop",
        source={"file": "x.json", "sha256": "abc", "kind": "sop_json",
                "acquired_from": None, "rights": None},
        book=meta,
        id_rule="sop/plain",
        alignment=None,
        stats={"blocks_in": 2, "blocks_out": 2, "dropped": 0, "damage": 0.0,
               "words": 7, "dropped_detail": []},
        blocks=blocks,
    )


class DumpLoadRoundTrip(unittest.TestCase):
    def test_round_trip_preserves_content(self):
        book = _plain_sop_book()
        with tempfile.TemporaryDirectory() as td:
            path = Path(td) / "book.json"
            dump(book, path)
            self.assertTrue(path.exists())
            back = load(path)
        self.assertEqual(back.schema, book.schema)
        self.assertEqual(back.profile, book.profile)
        self.assertEqual(back.book, book.book)
        self.assertEqual(back.id_rule, book.id_rule)
        self.assertEqual(back.alignment, book.alignment)
        self.assertEqual(back.stats, book.stats)
        self.assertEqual(back.blocks, book.blocks)

    def test_dump_has_stable_key_order_and_indent(self):
        book = _plain_sop_book()
        with tempfile.TemporaryDirectory() as td:
            path = Path(td) / "book.json"
            dump(book, path)
            raw = path.read_text(encoding="utf-8")
        doc = json.loads(raw)
        self.assertEqual(list(doc.keys()),
                          ["schema", "profile", "source", "book", "id_rule",
                           "alignment", "stats", "blocks"])
        self.assertIn("\n  ", raw)  # indent=2
        block_keys = list(doc["blocks"][0].keys())
        self.assertEqual(block_keys,
                          ["para_key", "page", "para", "seq", "chunks", "text", "words"])

    def test_load_rejects_bad_json(self):
        with tempfile.TemporaryDirectory() as td:
            path = Path(td) / "book.json"
            path.write_text("{not json", encoding="utf-8")
            with self.assertRaises(BookError):
                load(path)

    def test_load_rejects_missing_top_level_key(self):
        with tempfile.TemporaryDirectory() as td:
            path = Path(td) / "book.json"
            doc = {"schema": contract.SCHEMA_BOOK, "profile": "sop"}
            path.write_text(json.dumps(doc), encoding="utf-8")
            with self.assertRaises(BookError):
                load(path)

    def test_load_rejects_unknown_profile(self):
        with tempfile.TemporaryDirectory() as td:
            path = Path(td) / "book.json"
            book = _plain_sop_book(book_code=None)
            dump(book, path)
            doc = json.loads(path.read_text(encoding="utf-8"))
            doc["profile"] = "nonsense"
            path.write_text(json.dumps(doc), encoding="utf-8")
            with self.assertRaises(BookError):
                load(path)

    def test_load_rejects_malformed_block(self):
        with tempfile.TemporaryDirectory() as td:
            path = Path(td) / "book.json"
            book = _plain_sop_book()
            dump(book, path)
            doc = json.loads(path.read_text(encoding="utf-8"))
            del doc["blocks"][0]["words"]
            path.write_text(json.dumps(doc), encoding="utf-8")
            with self.assertRaises(BookError):
                load(path)

    def test_load_accepts_null_metadata(self):
        """extract never invents metadata (rule #9) — load() must not choke
        on a book.json whose book_code/author/etc. are still null; that is
        validate()'s job to report, not load()'s job to reject."""
        book = _plain_sop_book(book_code=None, title=None)
        with tempfile.TemporaryDirectory() as td:
            path = Path(td) / "book.json"
            dump(book, path)
            back = load(path)  # must not raise
        self.assertIsNone(back.book["book_code"])


class Validate(unittest.TestCase):
    def test_valid_book_has_no_errors(self):
        self.assertEqual(validate(_plain_sop_book()), [])

    def test_missing_required_metadata(self):
        book = _plain_sop_book(book_code=None, lang=None, title=None)
        errors = validate(book)
        self.assertTrue(any("book_code" in e for e in errors))
        self.assertTrue(any("lang" in e for e in errors))
        self.assertTrue(any("title" in e for e in errors))

    def test_egw_exempt_from_author_year(self):
        # corpus is None -> treated as EGW -> author/year not required
        book = _plain_sop_book(corpus=None, author=None, year=None)
        errors = validate(book)
        self.assertFalse(any("author" in e for e in errors))
        self.assertFalse(any("year" in e for e in errors))

    def test_non_egw_requires_author_year(self):
        book = _plain_sop_book(corpus="pioneers", author=None, year=None)
        errors = validate(book)
        self.assertTrue(any("author" in e for e in errors))
        self.assertTrue(any("year" in e for e in errors))

    def test_non_egw_with_author_year_is_clean(self):
        book = _plain_sop_book(corpus="pioneers", author="J. N. Andrews", year=1873)
        self.assertEqual(validate(book), [])

    def test_rejects_bad_id_rule_for_profile(self):
        book = _plain_sop_book()
        book.id_rule = "bible/v1"
        errors = validate(book)
        self.assertTrue(any("id_rule" in e for e in errors))

    def test_rejects_duplicate_para_key_seq(self):
        book = _plain_sop_book()
        book.blocks.append(Block(para_key="1.1", page=1, para=1, seq=0, chunks=1,
                                  text="duplicate", words=1))
        errors = validate(book)
        self.assertTrue(any("duplicate" in e for e in errors))

    def test_rejects_empty_text(self):
        book = _plain_sop_book()
        book.blocks[0].text = "   "
        errors = validate(book)
        self.assertTrue(any("empty text" in e for e in errors))

    def test_rejects_chunks_seq_inconsistency_gap(self):
        book = _plain_sop_book()
        # A split block that skips seq 1 (claims chunks=3 but only 0 and 2 exist)
        book.blocks = [
            Block(para_key="9.1", page=9, para=1, seq=0, chunks=3, text="part one here", words=3),
            Block(para_key="9.1", page=9, para=1, seq=2, chunks=3, text="part three here", words=3),
        ]
        errors = validate(book)
        self.assertTrue(any("9.1" in e and "seq" in e for e in errors))

    def test_rejects_inconsistent_chunks_value(self):
        book = _plain_sop_book()
        book.blocks = [
            Block(para_key="9.1", page=9, para=1, seq=0, chunks=2, text="part one here", words=3),
            Block(para_key="9.1", page=9, para=1, seq=1, chunks=3, text="part two here", words=3),
        ]
        errors = validate(book)
        self.assertTrue(any("inconsistent" in e for e in errors))

    def test_valid_chunked_block_is_clean(self):
        book = _plain_sop_book()
        book.blocks = [
            Block(para_key="9.1", page=9, para=1, seq=0, chunks=2, text="part one here", words=3),
            Block(para_key="9.1", page=9, para=1, seq=1, chunks=2, text="part two here", words=3),
        ]
        self.assertEqual(validate(book), [])


class Payload(unittest.TestCase):
    def test_sop_payload_matches_contract(self):
        book = _plain_sop_book()
        profile = contract.get_profile("sop")
        for block in book.blocks:
            payload = to_payload(book, block)
            self.assertEqual(contract.validate_payload(profile, payload), [])

    def test_sop_payload_carries_required_keys_even_when_none(self):
        book = _plain_sop_book(book_pair=None)
        payload = to_payload(book, book.blocks[0])
        self.assertIn("book_pair", payload)
        self.assertIsNone(payload["book_pair"])
        self.assertIn("aligned", payload)
        self.assertIsNone(payload["aligned"])

    def test_sop_payload_omits_absent_optional_keys(self):
        book = _plain_sop_book(author=None, title="Some Title", year=None, slug=None,
                                page_kind=None, corpus=None)
        payload = to_payload(book, book.blocks[0])
        self.assertNotIn("author", payload)
        self.assertNotIn("year", payload)
        self.assertNotIn("slug", payload)
        self.assertNotIn("page_kind", payload)
        self.assertNotIn("corpus", payload)
        self.assertEqual(payload["title"], "Some Title")

    def test_sop_payload_chunk_fields_only_when_split(self):
        book = _plain_sop_book()
        unsplit = Block(para_key="2.1", page=2, para=1, seq=0, chunks=1, text="one piece", words=2)
        split = Block(para_key="2.2", page=2, para=2, seq=1, chunks=2, text="second piece", words=2)
        p1 = to_payload(book, unsplit)
        p2 = to_payload(book, split)
        self.assertNotIn("chunk", p1)
        self.assertNotIn("chunks", p1)
        self.assertEqual(p2["chunk"], 1)
        self.assertEqual(p2["chunks"], 2)

    def test_aligned_pulled_from_alignment_en_reverse(self):
        # en_reverse is keyed by EN para_key -> [own-language para_keys] (the
        # sop_json shape); a DE book inverts it to go from its own para_key
        # back to the aligned EN para_key(s).
        book = _plain_sop_book(lang="de", book_code="BH")
        book.alignment = {"en_code": "SL", "en_reverse": {"7.1": ["1.1"]}}
        payload = to_payload(book, book.blocks[0])  # para_key "1.1"
        self.assertEqual(payload["aligned"], ["7.1"])
        payload2 = to_payload(book, book.blocks[1])  # "1.2" has no alignment entry
        self.assertIsNone(payload2["aligned"])

    def test_aligned_direct_lookup_for_english_book(self):
        book = _plain_sop_book(lang="en", book_code="SL")
        book.alignment = {"en_code": "SL", "en_reverse": {"1.1": ["5.1"]}}
        payload = to_payload(book, book.blocks[0])  # para_key "1.1"
        self.assertEqual(payload["aligned"], ["5.1"])

    def test_bible_payload_matches_contract(self):
        book = _plain_sop_book(book_code="KJV")
        book.profile = "bible"
        profile = contract.get_profile("bible")
        payload = to_payload(book, book.blocks[0])
        self.assertEqual(contract.validate_payload(profile, payload), [])


class Uid(unittest.TestCase):
    def test_uid_agrees_with_contract_for_sop_plain(self):
        book = _plain_sop_book(lang="en", book_code="ABC")
        block = book.blocks[0]
        got = uid(book, block)
        want = contract.uid_for("sop/plain", {"lang": "en", "book_code": "ABC", "para_key": "1.1"})
        self.assertEqual(got, want)

    def test_uid_agrees_with_contract_for_sop_seq(self):
        book = _plain_sop_book(lang="en", book_code="ABC")
        book.id_rule = "sop/seq"
        block = Block(para_key="3.4", page=3, para=4, seq=2, chunks=3, text="x y z", words=3)
        got = uid(book, block)
        want = contract.uid_for("sop/seq",
                                 {"lang": "en", "book_code": "ABC", "para_key": "3.4", "seq": 2})
        self.assertEqual(got, want)

    def test_uid_agrees_with_contract_for_bible(self):
        book = _plain_sop_book(book_code="KJV")
        book.profile = "bible"
        book.id_rule = "bible/v1"
        block = book.blocks[0]
        got = uid(book, block)
        want = contract.uid_for("bible/v1", {"bible": "KJV", "osis": block.para_key})
        self.assertEqual(got, want)

    def test_uid_feeds_a_stable_point_id(self):
        book = _plain_sop_book(lang="en", book_code="ABC")
        pid1 = contract.point_id(book.id_rule, {"lang": "en", "book_code": "ABC", "para_key": "1.1"})
        pid2 = contract.point_id(book.id_rule, {"lang": "en", "book_code": "ABC", "para_key": "1.1"})
        self.assertEqual(pid1, pid2)


if __name__ == "__main__":
    unittest.main()
