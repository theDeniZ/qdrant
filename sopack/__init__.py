"""sopack — corpus import pipeline for the bible-sop Qdrant collections.

Two halves that never run on the same machine:

* **Mac** — ``extract`` (source → reviewable book.json) and ``pack``
  (book.json → .sopack, the slow embedding step).
* **Server** — the import service in ``app/``, which validates and upserts a
  .sopack and never loads an embedding model.

``contract`` and ``format`` are shared by both and are stdlib-only. Everything
that needs fastembed lives in ``pack``.

See ``docs/IMPORT-PIPELINE-PLAN.md``.
"""

__version__ = "0.1.2"
