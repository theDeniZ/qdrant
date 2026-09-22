# Consolidating the book corpora into `qdrant/` under Git LFS

Every book source moves under one folder in this repo, tracked by Git LFS, so the
import pipeline and the material it imports live together.

**Nothing here is run for you.** Git is yours alone (see the project's hard rule), so
this document is the command list to run yourself. Nothing is deleted from its current
location by any command below — the copies are additive, exactly as asked.

---

## 1. What exists today

| Current location | Size | Files | What it is |
|---|---:|---:|---|
| `library/SoP Books DE/` | 66 MB | 92 | SoP German sources. **Extensionless** UTF-8 text (`xx02`…`xx93`), CRLF, with a `Buchtitel: / Buch-Code:` header. Input to `preprocess_sop_de.py`. |
| `library/SoP Books EN/` | 227 MB | 588 | SoP English sources, EPUB |
| `generator/data/sop/` | 317 MB | 713 json | The normalised corpus the pipeline reads (de/en/ja/ko + `book_map.json`). Already LFS-tracked *in the generator repo*. |
| `generator/data/bibles/` | 60 MB | 11 json | Bible translations |
| `qdrant/pd-books/ready/` | 35 MB | 42 epub | QA'd, de-imaged pioneer EPUBs |
| `qdrant/pd-books/converted/` | 18 MB | 70 epub | PDF→EPUB conversions |
| `qdrant/pd-books/cleaned/` | 7.4 MB | 11 epub | Running-header pass applied |
| `qdrant/pd-books/downloads/` | **2.3 GB** | 89 pdf + 45 epub | Original acquisitions, source of record |
| `qdrant/pd-books/ocr/` | 33 MB | 5802 txt | Intermediate OCR output — **derived, do not commit** |

> `qdrant/pd-books/.gitignore` is a single `*`, so **none of `pd-books/` is tracked
> today**. Leave that as it is: `pd-books/` stays the untracked working/scratch area,
> and the outputs worth keeping are copied into `books/` below.

### ⚠️ Check your LFS quota before pushing

GitHub's free tier is **1 GB of LFS storage and 1 GB/month of bandwidth**; each data
pack is 50 GB. The totals here:

- without `downloads/`: **≈ 730 MB** — fits the free tier, but with little room
- with `downloads/`: **≈ 3.0 GB** — needs at least one data pack

If the remote is self-hosted (Gitea, GitLab, …) none of this applies. My
recommendation: start without `downloads/`, and add it once you have decided about a
data pack. The originals are already safe on disk and on archive.org.

---

## 2. Target layout

```
qdrant/books/
├── sources/              originals as acquired — the source of record
│   ├── sop-de/           ← library/SoP Books DE          (extensionless text)
│   ├── sop-en/           ← library/SoP Books EN          (EPUB)
│   └── pioneers/         ← pd-books/downloads            (optional, 2.3 GB)
├── editions/             QA'd, publishable files
│   └── pioneers/         ← pd-books/{ready,converted,cleaned}
├── corpora/              normalised JSON the pipeline reads
│   ├── sop/              ← generator/data/sop
│   └── bibles/           ← generator/data/bibles
└── prepared/             book.json from `sopack extract`   ← NOT in LFS, on purpose
```

`prepared/` is deliberately outside LFS: its whole reason to exist is that it is
reviewable and diffable, and **LFS-tracked files do not show diffs**. They are JSON
text and compress well. If they ever become a problem, that is the moment to move them
in — not before.

Build artifacts (`*.sopack`) are never committed.

---

## 3. Commands

### 3.1 Install Git LFS

```bash
# macOS
brew install git-lfs

# the devcontainer (Debian)
sudo apt-get update && sudo apt-get install -y git-lfs

git lfs install          # once per machine, sets up the filters
git lfs version
```

### 3.2 Set up tracking — **before copying any file in**

This order matters. A file committed before `.gitattributes` exists goes in as an
ordinary blob and stays in history forever; fixing that afterwards needs
`git lfs migrate import --everything`, which rewrites history.

```bash
cd /workspaces/sdarm/qdrant

cat > .gitattributes <<'EOF'
# Book corpora — everything under books/ is LFS, EXCEPT prepared/ which must
# stay diffable (LFS files show no diff). Patterns are directory-scoped rather
# than extension-scoped because the German SoP sources have NO extension.
books/sources/**    filter=lfs diff=lfs merge=lfs -text
books/editions/**   filter=lfs diff=lfs merge=lfs -text
books/corpora/**    filter=lfs diff=lfs merge=lfs -text

# Keep these out of LFS and readable
books/prepared/**   -filter -diff -merge text=auto
books/**/README.md  -filter -diff -merge text=auto
.gitattributes      -filter -diff -merge text=auto
EOF

cat >> .gitignore <<'EOF'

# Import pipeline build artifacts — never committed
books/packs/
*.sopack
EOF

git add .gitattributes .gitignore
git commit -m "books: track corpora under Git LFS"
```

### 3.3 Verify the filter is armed before moving 3 GB

```bash
mkdir -p books/sources/sop-en
cp "../library/SoP Books EN/101Q.epub" books/sources/sop-en/
git add books/sources/sop-en/101Q.epub
git check-attr filter -- books/sources/sop-en/101Q.epub   # must say: filter: lfs
git lfs status                                            # must list it under LFS
```

If `check-attr` does not say `filter: lfs`, stop — do not continue until it does.

### 3.4 Copy the corpora in

```bash
cd /workspaces/sdarm/qdrant
mkdir -p books/{sources/{sop-de,sop-en,pioneers},editions/pioneers,corpora,prepared}

# SoP sources (note the quoted paths — the directories contain spaces)
rsync -a --info=progress2 "../library/SoP Books DE/"  books/sources/sop-de/
rsync -a --info=progress2 "../library/SoP Books EN/"  books/sources/sop-en/

# Normalised corpora
rsync -a --info=progress2 ../generator/data/sop/      books/corpora/sop/
rsync -a --info=progress2 ../generator/data/bibles/   books/corpora/bibles/

# Pioneer editions — the publishable ones, not the OCR intermediates
rsync -a --info=progress2 --include='*/' --include='*.epub' --exclude='*' \
      pd-books/ready/      books/editions/pioneers/ready/
rsync -a --info=progress2 --include='*/' --include='*.epub' --exclude='*' \
      pd-books/converted/  books/editions/pioneers/converted/
rsync -a --info=progress2 --include='*/' --include='*.epub' --exclude='*' \
      pd-books/cleaned/    books/editions/pioneers/cleaned/

# Optional, 2.3 GB — only with a data pack in place
# rsync -a --info=progress2 pd-books/downloads/ books/sources/pioneers/
```

The `generator/data/sop` files are LFS objects in the *generator* repo. `rsync` copies
the real content (the filter smudges on checkout), so what lands is the actual JSON —
verify with `head -c 100 books/corpora/sop/en/DA.json`; if you see
`version https://git-lfs.github.com/spec/v1` you copied pointer files and need
`git lfs pull` in the generator repo first.

### 3.5 Commit in batches

3 GB in one commit is a bad time. Split it, so a failed push costs one batch:

```bash
cd /workspaces/sdarm/qdrant

git add books/corpora/bibles   && git commit -m "books: bible corpora"
git add books/corpora/sop      && git commit -m "books: SoP corpora (de/en/ja/ko)"
git add books/sources/sop-de   && git commit -m "books: SoP German sources"
git add books/sources/sop-en   && git commit -m "books: SoP English sources"
git add books/editions         && git commit -m "books: pioneer editions"

git lfs ls-files | wc -l       # how many objects are in LFS
git count-objects -vH          # git's own size — should stay small
git push                       # uploads the LFS objects
```

### 3.6 After cloning elsewhere

```bash
git clone <url> && cd qdrant && git lfs pull
# or, to skip the 3 GB and fetch lazily:
GIT_LFS_SKIP_SMUDGE=1 git clone <url>
```

---

## 4. Things that will bite

- **`.gitattributes` must be committed first.** Covered in 3.2; it is the only
  irreversible mistake in this list.
- **Quoted paths.** `library/SoP Books DE` contains spaces.
- **The German sources have no extension.** Extension patterns (`*.epub`) silently miss
  all 92 of them. The patterns above are directory-scoped for exactly this reason.
- **`pd-books/ocr/` (5802 txt) is derived** from `downloads/`. Committing it costs
  33 MB and 5802 LFS objects for something regenerable. Excluded above.
- **Don't add `pd-books/` to git.** Its `.gitignore` of `*` is doing useful work; the
  keepers are copied into `books/` instead.
- **LFS bandwidth is metered on clone, not just push.** CI that clones this repo will
  burn the monthly allowance unless it sets `GIT_LFS_SKIP_SMUDGE=1`.

## 5. Once the files are in

`books/corpora/sop` and `books/corpora/bibles` become the inputs to
`sopack extract --kind sop_json`, and `books/sources` / `books/editions` the inputs to
`sopack extract --kind epub`. See [IMPORT-PIPELINE-PLAN.md](IMPORT-PIPELINE-PLAN.md) §3.

Moving the generator's copies is a separate decision and is **not** part of the commands
above — `generator/data/sop` and `generator/data/bibles` keep working exactly as they do
now, and the generator's own `.gitattributes` is untouched.
