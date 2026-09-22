#!/usr/bin/env python3
"""Unit tests for sopack.extract — epub/markdown/text/sop_json extractors and
the shared chunker/damage gates.

Run:  /workspaces/sdarm/.venv/bin/python3.11 sopack/tests/test_extract.py
"""
from __future__ import annotations

import io
import json
import sys
import tempfile
import unittest
import zipfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))  # qdrant/

from sopack import contract
from sopack.book import BookError, validate
from sopack.extract import chunk, extract
from sopack.extract import epub as epub_mod
from sopack.extract import sop_json as sop_json_mod


# ── tiny in-memory EPUB fixture builder ──────────────────────────────────────

_CONTAINER_XML = """<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"""


def _opf(title="Test Work", creator="A. Author", date="1873", items=("chap1.xhtml",)):
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


def make_epub(path: Path, chapters: list[str], title="Test Work", creator="A. Author",
              date="1873"):
    """*chapters* are raw XHTML body fragments, one per spine document."""
    names = [f"chap{i}.xhtml" for i in range(len(chapters))]
    with zipfile.ZipFile(path, "w") as zf:
        zf.writestr("mimetype", "application/epub+zip")
        zf.writestr("META-INF/container.xml", _CONTAINER_XML)
        zf.writestr("OEBPS/content.opf", _opf(title, creator, date, names))
        for name, body in zip(names, chapters):
            zf.writestr(f"OEBPS/{name}",
                        f'<?xml version="1.0"?><html><body>{body}</body></html>')


class ChunkBoundaries(unittest.TestCase):
    def test_short_text_not_split(self):
        text = "This is a short paragraph of prose."
        self.assertEqual(chunk.split_long(text), [text])

    def test_long_text_is_split_on_sentence_boundaries(self):
        sentence = "This is one sentence with several words in it. "
        text = sentence * 40  # well over MAX_WORDS
        pieces = chunk.split_long(text)
        self.assertGreater(len(pieces), 1)
        for p in pieces:
            self.assertLessEqual(len(p.split()), chunk.MAX_WORDS)
        # nothing lost
        self.assertEqual(
            len(" ".join(pieces).split()),
            len(text.split()),
        )

    def test_single_sentence_longer_than_max_words_is_hard_cut(self):
        text = "word " * (chunk.MAX_WORDS + 50)
        pieces = chunk.split_long(text.strip())
        self.assertGreater(len(pieces), 1)
        for p in pieces:
            self.assertLessEqual(len(p.split()), chunk.MAX_WORDS)

    def test_quality_gate_drops_too_short(self):
        reason = chunk.quality_gate("Too short.")
        self.assertIsNotNone(reason)
        self.assertIn("short", reason)

    def test_quality_gate_passes_normal_prose(self):
        text = "This is a perfectly ordinary sentence of nineteenth century prose about faith."
        self.assertIsNone(chunk.quality_gate(text))


class DamageGate(unittest.TestCase):
    def test_clean_prose_scores_low_damage(self):
        text = "The truth of the gospel shines through every generation of believers."
        self.assertLess(chunk.damage_score(text), 0.1)

    def test_scrambled_text_scores_high_damage(self):
        text = "^^i'^M - er^ aniel anb IRrt^flati oNs^ ButastoJesus xQz^^ fW~rD"
        self.assertGreater(chunk.damage_score(text), chunk.MAX_BLOCK_DAMAGE)

    def test_junk_char_ratio_flags_box_drawing_noise(self):
        text = "░▒▓" * 20
        self.assertGreater(chunk.junk_char_ratio(text), chunk.MAX_JUNK_CHARS)

    def test_junk_char_ratio_clean_for_ordinary_text(self):
        text = "Ordinary English prose, with punctuation; and a hyphen-ated word."
        self.assertLess(chunk.junk_char_ratio(text), chunk.MAX_JUNK_CHARS)


class BoilerplateStripping(unittest.TestCase):
    def test_archive_org_disclaimer_matches(self):
        text = "This book was produced in EPUB format by the Internet Archive."
        self.assertTrue(epub_mod.BOILERPLATE_RE.search(text))

    def test_gutenberg_credit_matches(self):
        text = "Produced by the Online Distributed Proofreading Team at pgdp.net"
        self.assertTrue(epub_mod.BOILERPLATE_RE.search(text))

    def test_ordinary_prose_does_not_match(self):
        text = "In the beginning God created the heavens and the earth."
        self.assertFalse(epub_mod.BOILERPLATE_RE.search(text))

    def test_numeric_toc_line_detected(self):
        tokens = "1 2 3 4 5 6 7 8 9 10 Contents Page".split()
        numeric_hits = sum(bool(epub_mod.NUMERIC_RE.match(t)) for t in tokens)
        self.assertGreater(numeric_hits, 0.35 * len(tokens))


class RefLifting(unittest.TestCase):
    def test_ref_re_lifts_trailing_page_para(self):
        m = epub_mod.REF_RE.search("Some quoted sentence of the book. CHR 24.1")
        self.assertIsNotNone(m)
        self.assertEqual(m.group(1), "CHR")
        self.assertEqual(m.group(2), "24")
        self.assertEqual(m.group(3), "1")

    def test_ref_re_does_not_match_plain_prose(self):
        self.assertIsNone(epub_mod.REF_RE.search("This sentence has no trailing citation."))

    def test_pagemark_re_matches_bare_bracketed_number(self):
        m = epub_mod.PAGEMARK_RE.match("[89]")
        self.assertIsNotNone(m)
        self.assertEqual(m.group(1), "89")

    def test_pagemark_re_does_not_match_prose(self):
        self.assertIsNone(epub_mod.PAGEMARK_RE.match("[not a page marker]"))


class EpubExtraction(unittest.TestCase):
    def _long_para(self, n=40):
        return ("This sentence has a handful of words in it for length. " * n).strip()

    def test_basic_extraction_reads_opf_metadata_and_paragraphs(self):
        with tempfile.TemporaryDirectory() as td:
            path = Path(td) / "book.epub"
            make_epub(path, [
                "<p>This is the first paragraph of real prose in the book.</p>"
                "<p>This is the second paragraph, also real prose, quite readable.</p>",
            ], title="History of Something", creator="J. N. Andrews", date="1873")
            book = extract(path, "epub", book_code="HSFD", corpus="pioneers", slug="hsfd")

        self.assertEqual(book.profile, "sop")
        self.assertEqual(book.id_rule, "sop/seq")
        self.assertEqual(book.book["book_code"], "HSFD")
        self.assertEqual(book.book["title"], "History of Something")
        self.assertEqual(book.book["author"], "J. N. Andrews")
        self.assertEqual(book.book["year"], 1873)
        self.assertEqual(book.book["book_pair"], "HSFD")
        self.assertEqual(len(book.blocks), 2)
        self.assertEqual(book.stats["blocks_in"], 2)
        self.assertEqual(book.stats["blocks_out"], 2)
        self.assertEqual(book.stats["dropped"], 0)
        self.assertEqual(validate(book), [])

    def test_boilerplate_paragraph_is_dropped_and_recorded(self):
        with tempfile.TemporaryDirectory() as td:
            path = Path(td) / "book.epub"
            make_epub(path, [
                "<p>This book was produced in EPUB format by the Internet Archive, "
                "which relies on optical character recognition software.</p>"
                "<p>This is genuine prose that should survive the boilerplate filter fine.</p>",
            ])
            book = extract(path, "epub", book_code="X", corpus="pioneers",
                            author="A", year=1900, slug="x")

        self.assertEqual(len(book.blocks), 1)
        self.assertEqual(book.stats["dropped"], 1)
        reasons = [d["reason"] for d in book.stats["dropped_detail"]]
        self.assertTrue(any("boilerplate" in r for r in reasons))

    def test_inline_ref_lifts_page_para_and_strips_citation(self):
        blocks_html = "".join(
            f"<p>Sentence number {i} of real readable prose in this book. CHR {i}.1</p>"
            for i in range(1, 6)
        )
        with tempfile.TemporaryDirectory() as td:
            path = Path(td) / "book.epub"
            make_epub(path, [blocks_html])
            book = extract(path, "epub", corpus="pioneers", author="A", year=1900, slug="x")

        self.assertEqual(book.book["book_code"], "CHR")
        self.assertEqual(book.book["page_kind"], "print")
        keys = sorted(b.para_key for b in book.blocks)
        self.assertEqual(keys, [f"{i}.1" for i in range(1, 6)])
        for b in book.blocks:
            self.assertNotIn("CHR", b.text)

    def test_pagemark_drives_page_para_when_no_inline_code(self):
        html = (
            "<p>[12]</p>"
            "<p>This is the first real paragraph of prose on printed page twelve.</p>"
            "<p>This is the second real paragraph of prose on the very same page.</p>"
            "<p>[13]</p>"
            "<p>This is a paragraph of prose that begins printed page thirteen instead.</p>"
        )
        with tempfile.TemporaryDirectory() as td:
            path = Path(td) / "book.epub"
            make_epub(path, [html])
            book = extract(path, "epub", book_code="Z", corpus="pioneers",
                            author="A", year=1900, slug="z")

        keys = sorted(b.para_key for b in book.blocks)
        self.assertEqual(keys, ["12.1", "12.2", "13.1"])
        self.assertEqual(book.book["page_kind"], "print")

    def test_over_long_paragraph_is_chunked(self):
        with tempfile.TemporaryDirectory() as td:
            path = Path(td) / "book.epub"
            make_epub(path, [f"<p>{self._long_para()}</p>"])
            book = extract(path, "epub", book_code="LNG", corpus="pioneers",
                            author="A", year=1900, slug="lng")

        self.assertGreater(len(book.blocks), 1)
        for b in book.blocks:
            self.assertLessEqual(b.words, chunk.MAX_WORDS)
        self.assertEqual({b.chunks for b in book.blocks}, {len(book.blocks)})
        self.assertEqual(sorted(b.seq for b in book.blocks), list(range(len(book.blocks))))
        self.assertEqual(validate(book), [])

    def test_scrambled_title_page_is_dropped_by_damage_gate(self):
        html = (
            "<p>^^i'^M - er^ aniel anb IRrt^flati oNs^ ButastoJesus xQz^^ fW~rD garbled text</p>"
            "<p>This is a perfectly normal paragraph of readable nineteenth century prose text.</p>"
        )
        with tempfile.TemporaryDirectory() as td:
            path = Path(td) / "book.epub"
            make_epub(path, [html])
            book = extract(path, "epub", book_code="DAR", corpus="pioneers",
                            author="A", year=1900, slug="dar")

        self.assertEqual(len(book.blocks), 1)
        self.assertEqual(book.stats["dropped"], 1)

    def test_missing_file_raises(self):
        with self.assertRaises(BookError):
            extract(Path("/no/such/file.epub"), "epub")

    def test_book_code_left_null_when_not_derivable(self):
        with tempfile.TemporaryDirectory() as td:
            path = Path(td) / "book.epub"
            make_epub(path, ["<p>Ordinary prose with no inline citation code at all here.</p>"])
            book = extract(path, "epub")  # no book_code given, no inline code present
        self.assertIsNone(book.book["book_code"])
        errors = validate(book)
        self.assertTrue(any("book_code" in e for e in errors))


class SopJsonExtraction(unittest.TestCase):
    def test_english_only_file(self):
        doc = {
            "meta": {"en_code": "ABC", "en_title": "A Book", "publisher": "White Estate",
                     "year": "", "en_pages": 10},
            "en": {
                "0.1": {"text": "This is the first paragraph of the English original.",
                        "chapter": "Preface"},
                "0.2": {"text": "This is the second paragraph, also part of the preface.",
                        "chapter": "Preface"},
            },
        }
        with tempfile.TemporaryDirectory() as td:
            root = Path(td) / "en"
            root.mkdir()
            path = root / "ABC.json"
            path.write_text(json.dumps(doc), encoding="utf-8")
            book = extract(path, "sop_json")

        self.assertEqual(book.profile, "sop")
        self.assertEqual(book.id_rule, "sop/plain")
        self.assertEqual(book.book["lang"], "en")
        self.assertEqual(book.book["book_code"], "ABC")
        self.assertEqual(book.book["title"], "A Book")
        self.assertIsNone(book.book["corpus"])
        self.assertIsNone(book.alignment)
        self.assertEqual(len(book.blocks), 2)
        for b in book.blocks:
            self.assertEqual(b.chunks, 1)
            self.assertEqual(b.seq, 0)
        # EGW work: no corpus -> validate() does not require author/year
        self.assertEqual(validate(book), [])

    def test_german_file_carries_en_reverse_into_alignment(self):
        doc = {
            "meta": {"de_code": "BH", "en_code": "SL", "de_title": "Biblische Heiligung",
                     "en_title": "The Sanctified Life", "publisher": "Advent-Verlag",
                     "year": "1973", "de_pages": 61, "german_origin": False},
            "de": {
                "5.1": {"text": "Heiligung im Sinne der Bibel umfasst den ganzen Menschen.",
                        "en_ref": "7.1", "chapter": "Kapitel 1"},
                "5.2": {"text": "In der religiösen Welt herrscht eine falsche Heiligungslehre.",
                        "en_ref": "7.2", "chapter": "Kapitel 1"},
            },
            "en_reverse": {"7.1": ["5.1"], "7.2": ["5.2"]},
        }
        with tempfile.TemporaryDirectory() as td:
            root = Path(td) / "de"
            root.mkdir()
            path = root / "BH.json"
            path.write_text(json.dumps(doc), encoding="utf-8")
            book = extract(path, "sop_json")

        self.assertEqual(book.book["lang"], "de")
        self.assertEqual(book.book["book_code"], "BH")
        self.assertIsNotNone(book.alignment)
        self.assertEqual(book.alignment["en_code"], "SL")
        self.assertEqual(book.alignment["en_reverse"], doc["en_reverse"])

        # round-trip through to_payload: the aligned field must reflect en_reverse
        from sopack.book import to_payload
        b1 = next(b for b in book.blocks if b.para_key == "5.1")
        payload = to_payload(book, b1)
        self.assertEqual(payload["aligned"], ["7.1"])

    def test_empty_paragraph_text_is_dropped(self):
        doc = {
            "meta": {"en_code": "ABC", "en_title": "A Book"},
            "en": {
                "0.1": {"text": "   ", "chapter": "Preface"},
                "0.2": {"text": "This paragraph actually has content in it.", "chapter": "Preface"},
            },
        }
        with tempfile.TemporaryDirectory() as td:
            root = Path(td) / "en"
            root.mkdir()
            path = root / "ABC.json"
            path.write_text(json.dumps(doc), encoding="utf-8")
            book = extract(path, "sop_json")

        self.assertEqual(len(book.blocks), 1)
        self.assertEqual(book.stats["dropped"], 1)
        self.assertEqual(book.stats["dropped_detail"][0]["reason"], "empty text")

    def test_missing_file_raises(self):
        with self.assertRaises(BookError):
            extract(Path("/no/such/file.json"), "sop_json")

    def test_bad_json_raises(self):
        with tempfile.TemporaryDirectory() as td:
            path = Path(td) / "bad.json"
            path.write_text("{not json", encoding="utf-8")
            with self.assertRaises(BookError):
                extract(path, "sop_json")


class MarkdownAndTextExtraction(unittest.TestCase):
    def test_markdown_headings_bump_page_and_reset_para(self):
        md = (
            "# Chapter One\n\n"
            "This is the first paragraph of chapter one, with plenty of words to pass gates.\n\n"
            "This is the second paragraph of chapter one, likewise readable prose here.\n\n"
            "# Chapter Two\n\n"
            "This is the first paragraph of chapter two, again with enough words in it.\n"
        )
        with tempfile.TemporaryDirectory() as td:
            path = Path(td) / "book.md"
            path.write_text(md, encoding="utf-8")
            book = extract(path, "markdown", book_code="MD1", lang="en", title="T",
                            author="A", year=2020, corpus="test", slug="md1")

        keys = sorted(b.para_key for b in book.blocks)
        self.assertEqual(keys, ["2.1", "2.2", "3.1"])
        self.assertEqual(validate(book), [])

    def test_text_paragraphs_split_on_blank_lines(self):
        txt = (
            "This is the first paragraph of plain text with enough words in it to pass.\n\n"
            "This is the second paragraph of plain text, also long enough to survive.\n"
        )
        with tempfile.TemporaryDirectory() as td:
            path = Path(td) / "book.txt"
            path.write_text(txt, encoding="utf-8")
            book = extract(path, "text", book_code="TX1", lang="en", title="T",
                            author="A", year=2020, corpus="test", slug="tx1")

        self.assertEqual(len(book.blocks), 2)
        self.assertTrue(all(b.page == 1 for b in book.blocks))
        self.assertEqual(validate(book), [])

    def test_unknown_kind_raises(self):
        with self.assertRaises(BookError):
            extract(Path("/tmp/whatever"), "pdf")


class PayloadAndUidAgreement(unittest.TestCase):
    """Every extractor's output must produce payloads contract.validate_payload
    accepts, and uid()s that agree with contract.uid_for."""

    def _assert_book_payloads_and_uids_valid(self, book):
        from sopack.book import to_payload, uid as book_uid
        profile = contract.get_profile(book.profile)
        for block in book.blocks:
            payload = to_payload(book, block)
            errors = contract.validate_payload(profile, payload)
            self.assertEqual(errors, [], f"{block.para_key}: {errors}")
            u = book_uid(book, block)
            fields = dict(payload)
            if "#" not in u:
                fields.setdefault("seq", 0)
            # Recompute via contract directly using the same field shape book.py uses.
            if book.id_rule == "sop/seq":
                want_fields = {"lang": book.book["lang"], "book_code": book.book["book_code"],
                                "para_key": block.para_key, "seq": block.seq}
            elif book.id_rule == "sop/plain":
                want_fields = {"lang": book.book["lang"], "book_code": book.book["book_code"],
                                "para_key": block.para_key}
            else:
                self.fail(f"unexpected id_rule {book.id_rule!r}")
            self.assertEqual(u, contract.uid_for(book.id_rule, want_fields))

    def test_epub_book(self):
        with tempfile.TemporaryDirectory() as td:
            path = Path(td) / "book.epub"
            make_epub(path, [
                "<p>This is the first paragraph of real prose in the book.</p>"
                "<p>This is the second paragraph, also real prose, quite readable.</p>",
            ])
            book = extract(path, "epub", book_code="PAY", corpus="pioneers",
                            author="A", year=1900, slug="pay")
        self._assert_book_payloads_and_uids_valid(book)

    def test_sop_json_book(self):
        doc = {
            "meta": {"en_code": "PAY2", "en_title": "A Book"},
            "en": {"0.1": {"text": "This is a paragraph of real EGW prose for the test."}},
        }
        with tempfile.TemporaryDirectory() as td:
            root = Path(td) / "en"
            root.mkdir()
            path = root / "PAY2.json"
            path.write_text(json.dumps(doc), encoding="utf-8")
            book = extract(path, "sop_json")
        self._assert_book_payloads_and_uids_valid(book)


if __name__ == "__main__":
    unittest.main()
