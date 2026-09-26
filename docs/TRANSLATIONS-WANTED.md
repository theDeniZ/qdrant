# Translated Ellen G. White editions still missing from the corpus

Written 2026-09-26 for the SBL quarter-translation pipeline (`sdarm qt`, see
`generator/docs/TRANSLATION-PIPELINE.md`). Every SoP quote whose book has no
edition in the target language becomes a **fresh translation** (marked), so each
book imported here turns those quotes into verbatim published wording, found by
the pipeline automatically with no code change.

**How this was measured.** SoP citations (`note.sop.ref`) in all 28 official EN
quarters en-2020-1 … en-2026-4 were counted per book, and per language checked against
the live `sop` collection (`facet book_code` per `lang`; DE is keyed by DE code and
mapped through the edition's `en_code`). Corpus size on that day, in books:
de 68, ru 49, uk 7, es 49, ro 38, ja 8, ko 44.

**Where to find the missing editions.** The official RU/UK quarters print these
books under published titles (right-hand columns, mined from the official editions),
so the editions exist. Look for them wherever the language's publisher (or the
EGW Estate's language sites) makes them available, then import them with the sopack
pipeline (`corpus-prep` skill). Page numbering: RU/UK editions are cited with the
**English** page numbers, so an import must carry EN `page.para` keys
(`sop_book_paragraphs(lang=…)` relies on this).

## Priority for Ukrainian (7 books today: AA, COL, DA, GC, PP, SC, SR)

Ranked by citations in 2020–2026 quarters; periodicals left out (see below). The top ten would cover most UK quotes
that are fresh translations today (≈ 220 of 282 in 2026-4).

| # | EN code | book | cites | UK title the official quarters print |
|---|---|---|---|---|
| 1 | 5T | Testimonies for the Church | 202 | Свідоцтва для церкви |
| 2 | PK | Prophets and Kings | 135 | Пророки і царі |
| 3 | 4T | Testimonies for the Church | 118 | Свідоцтва для церкви |
| 4 | 2T | Testimonies for the Church | 111 | Свідоцтва для церкви |
| 5 | 6T | Testimonies for the Church | 109 | Свідоцтва для Церкви |
| 6 | MH | The Ministry of Healing | 94 | Служіння зцілення |
| 7 | Ed | Education | 91 | Виховання |
| 8 | EW | Early Writings | 85 | Ранні твори |
| 9 | 6BC | EGW SDA Bible Commentary | 85 | Біблійний коментар АСД [з коментарів Е.Г. Уайт] |
| 10 | 1T | Testimonies for the Church | 81 | Свідоцтва для церкви |
| 11 | TM | Testimonies to Ministers and Gospel Workers | 75 | Свідоцтва для проповідників |
| 12 | 3T | Testimonies for the Church | 72 | Свідоцтва для церкви |
| 13 | 7ABC | EGW SDA Bible Commentary | 66 | Біблійний коментар АСД [з коментарів Е.Г. Уайт] |
| 14 | MB | Thoughts From the Mount of Blessing | 65 | Блаженства, промовлені на горі |

## All languages — the 45 most-cited books

✓ = in the corpus, — = missing. Periodicals (RH, ST, YI, …) have no translated
editions. Their quotes are often reprinted in compilations, and the pipeline finds
those by vector search across all books of the language (cross-attribution), so
they are not listed as wanted editions.

| code | book | cites | quarters | de | ru | uk | es | ro | ja | ko | UK title | RU title |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| DA | The Desire of Ages | 788 | 28 | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | Бажання віків | Желание веков |
| PP | Patriarchs and Prophets | 572 | 24 | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | Патріархи і пророки | Патриархи и пророки |
| AA | The Acts of the Apostles | 374 | 26 | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | Дії апостолів | Деяния апостолов |
| GC | The Great Controversy | 274 | 28 | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | Велика боротьба | Великая борьба |
| 5T | Testimonies for the Church | 202 | 27 | ✓ | ✓ | — | ✓ | ✓ | — | ✓ | Свідоцтва для церкви | Свидетельства для Церкви |
| SC | Steps to Christ | 181 | 21 | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | Дорога до Христа | Путь ко Христу |
| RH | The Review and Herald, November 8, 1892. | 156 | 25 | — | — | — | — | — | — | — | Дивовижна Божа благодать |  |
| COL | Christ's Object Lessons | 148 | 26 | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | Наочні уроки Христа | Наглядные уроки Христа |
| PK | Prophets and Kings | 135 | 23 | ✓ | ✓ | — | ✓ | ✓ | ✓ | ✓ | Пророки і царі | Пророки и цари |
| 4T | Testimonies for the Church | 118 | 25 | ✓ | ✓ | — | ✓ | — | — | ✓ | Свідоцтва для церкви | Свидетельства для Церкви |
| 2T | Testimonies for the Church | 111 | 25 | ✓ | ✓ | — | ✓ | — | — | ✓ | Свідоцтва для церкви | Свидетельства для Церкви |
| 6T | Testimonies for the Church | 109 | 25 | ✓ | ✓ | — | ✓ | ✓ | — | ✓ | Свідоцтва для Церкви | Свидетельства для Церкви |
| ST | The Signs of the Times, August 24, 1891. | 97 | 23 | — | — | — | — | — | — | — |  |  |
| 1SM | Selected Messages | 96 | 21 | ✓ | ✓ | — | ✓ | ✓ | — | ✓ |  |  |
| MH | The Ministry of Healing | 94 | 26 | ✓ | ✓ | — | ✓ | ✓ | — | ✓ | Служіння зцілення | Служение исцеления |
| Ed | Education | 91 | 25 | ✓ | ✓ | — | ✓ | ✓ | — | ✓ | Виховання | Воспитание |
| EW | Early Writings | 85 | 24 | ✓ | ✓ | — | ✓ | ✓ | — | ✓ | Ранні твори | Ранние произведения |
| 6BC | EGW SDA Bible Commentary | 85 | 17 | ✓ | ✓ | — | — | — | — | — | Біблійний коментар АСД [з коментарів Е.Г. Уайт] | Библейский комментарий АСД [из комментариев Э.Г. Уайт] |
| 1T | Testimonies for the Church | 81 | 20 | ✓ | ✓ | — | ✓ | — | — | ✓ | Свідоцтва для церкви | Свидетельства для церкви |
| TM | Testimonies to Ministers and Gospel Workers | 75 | 24 | ✓ | ✓ | — | ✓ | ✓ | — | ✓ | Свідоцтва для проповідників | Свидетельства для проповедников |
| 3T | Testimonies for the Church | 72 | 19 | ✓ | ✓ | — | ✓ | — | — | ✓ | Свідоцтва для церкви | Свидетельства для Церкви |
| 7ABC | EGW SDA Bible Commentary | 66 | 19 | ✓ | — | — | — | — | — | — | Біблійний коментар АСД [з коментарів Е.Г. Уайт] | Библейский комментарий АСД [из комментариев Э.Г. Уайт] |
| MB | Thoughts From the Mount of Blessing | 65 | 20 | ✓ | ✓ | — | ✓ | ✓ | ✓ | ✓ | Блаженства, промовлені на горі | Блаженства, изреченные на горе |
| GW | Gospel Workers (1892) | 55 | 20 | ✓ | ✓ | — | ✓ | ✓ | — | ✓ | Служителі Євангелії | Служители Евангелия |
| Ev | Evangelism | 50 | 23 | ✓ | ✓ | — | ✓ | ✓ | — | ✓ | Євангелизм | Евангелизм |
| 9T | Testimonies for the Church | 49 | 18 | ✓ | ✓ | — | ✓ | — | — | ✓ | Свідоцтва для Церкви | Свидетельства для Церкви |
| CS | Counsels on Stewardship | 48 | 12 | — | — | — | — | — | — | — | Поради щодо управління ресурсами | Советы по управлению ресурсами |
| 8T | Testimonies for the Church | 48 | 19 | ✓ | ✓ | — | ✓ | ✓ | — | ✓ | Свідоцтва для церкви | Свидетельства для Церкви |
| AH | The Adventist Home | 47 | 15 | ✓ | ✓ | — | ✓ | ✓ | — | ✓ | Християнська родина | Христианский дом |
| FE | Fundamentals of Christian Education | 40 | 19 | — | — | — | — | — | — | — | Основи християнського виховання | Основы христианского воспитания |
| TMK | That I May Know Him | 36 | 15 | — | — | — | — | — | — | — | Щоб мені пізнати Його | Дабы мне познать Его |
| CT | Counsels to Parents, Teachers, and Students | 35 | 17 | — | — | — | — | — | — | — | Поради батькам, вчителям і студентам | Советы родителям, учителям и студентам |
| 2SM | Selected Messages | 35 | 20 | ✓ | ✓ | — | ✓ | — | — | ✓ | Свідоцтва для церкви |  |
| 1BC | EGW SDA Bible Commentary | 34 | 9 | ✓ | — | — | — | — | — | — | Біблійний коментар АСД [з коментарів Е.Г. Уайт] | Библейский комментарий АСД [из комментариев Э.Г. Уайт] |
| 5BC | EGW SDA Bible Commentary | 34 | 16 | ✓ | ✓ | — | — | — | — | — | Біблійний коментар АСД [з коментарів Е.Г. Уайт] | Библейский комментарий АСД [из комментариев Э.Г. Уайт] |
| 7T | Testimonies for the Church | 31 | 17 | ✓ | ✓ | — | ✓ | ✓ | — | ✓ | Свідоцтва для Церкви | Свидетельства для Церкви |
| OHC | Our High Calling | 30 | 17 | — | — | — | — | — | — | — | Наше найвище покликання | Наше высокое призвание |
| FLB | The Faith I Live By | 27 | 14 | — | — | — | — | — | — | — | Віра, якою я живу | Вера, которой я живу |
| FW | Faith and Works | 27 | 11 | ✓ | ✓ | — | ✓ | ✓ | — | ✓ | Віра і діла | Вера и дела |
| ML | My Life Today | 26 | 12 | — | — | — | — | — | — | — | Моє життя сьогодні | Моя жизнь сегодня |
| 3SG | Spiritual Gifts | 25 | 9 | — | — | — | — | — | — | — | Духовні дари | Духовные дары |
| CG | Child Guidance | 25 | 14 | ✓ | ✓ | — | ✓ | ✓ | — | ✓ | Виховання дітей | Воспитание детей |
| PH050 | Messages to Young People | 25 | 16 | — | — | — | — | — | — | — | Вісті для молоді | Вести для молодежи |
| HP | In Heavenly Places | 24 | 14 | — | — | — | — | — | — | — | У Небесних оселях | В Небесных обителях |
| SD | Sons and Daughters of God | 22 | 14 | — | — | — | — | — | — | — | Сини та доньки Бога | Сыновья и дочери Бога |

## Related

- `docs/FINDING-de-alignment.md`: the `aligned` payload is inverted. The translation
  pipeline no longer depends on it (it pairs paragraphs by stored vectors), but
  `sop_parallel` still does.
- Re-run the count after new quarters: the numbers come from the sbl server's archive
  (`archive_status`, MCP) and `facet book_code` per language.
