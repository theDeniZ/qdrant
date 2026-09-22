# Import API — wire contract

The interface between the import service (`app/import_service.py`, `app/jobs.py`)
and the admin UI (`app/admin.py`, `app/static/import.js`). Both sides build against
**this document**; neither invents a field.

All routes live in the **admin app** (`ADMIN_PORT`, default 8081), behind the existing
`_authorized()` HTTP Basic check and, for every mutating method, `_same_origin()`.
Responses are JSON unless stated. Errors are
`{"error": "<code>", "detail": "<human sentence>"}` with a 4xx/5xx status.

---

## 1. Upload

Packs are large by design, so the browser slices the file and no single request is
long. Parts must arrive in order; a gap is rejected.

| Method | Path | Body | Returns |
|---|---|---|---|
| `POST` | `/import/uploads` | `{"name": str, "size": int, "sha256": str}` | `{"upload_id": str, "part_size": int, "received": 0}` |
| `PUT` | `/import/uploads/{upload_id}/parts/{n}` | raw bytes (`application/octet-stream`), `n` is 0-based | `{"received": int, "bytes": int}` |
| `GET` | `/import/uploads/{upload_id}` | — | `{"upload_id", "name", "size", "bytes", "received", "part_size", "complete": bool}` |
| `POST` | `/import/uploads/{upload_id}/complete` | — | `{"pack_id": str, "manifest": {...}}` |
| `DELETE` | `/import/uploads/{upload_id}` | — | `204` |

- `part_size` is **8 MiB** (`8 * 1024 * 1024`).
- `complete` verifies total size and sha256 against what `POST /import/uploads`
  declared. Mismatch → `400 {"error": "checksum_mismatch"}` and the staging file is
  discarded.
- On success the file moves to `/data/packs/{pack_id}.sopack` and `complete` returns
  the parsed manifest so the UI can show what is about to be imported **before** a job
  is created.
- `GET` is what makes an interrupted upload resumable: the client resends from
  `received`.
- Uploads not completed within 24 h are pruned.

## 2. Packs

| Method | Path | Returns |
|---|---|---|
| `GET` | `/import/packs` | `{"packs": [{"pack_id", "name", "bytes", "uploaded_at", "profile", "points", "books", "imported_by": [job_id, …]}]}` |
| `GET` | `/import/packs/{pack_id}` | one pack incl. its full `manifest` |
| `DELETE` | `/import/packs/{pack_id}` | `204`; refuses with `409` if a job referencing it is `running` |

## 3. Jobs

| Method | Path | Body | Returns |
|---|---|---|---|
| `POST` | `/import/jobs` | `{"pack_id": str, "mode": "dry-run"\|"apply", "allow_overwrite": bool}` | `{"job_id": str}` (202) |
| `GET` | `/import/jobs` | — | `{"jobs": [<job summary>, …]}` newest first |
| `GET` | `/import/jobs/{job_id}` | — | `<job>` (full) |
| `GET` | `/import/jobs/{job_id}/log?after={seq}` | — | `{"events": [<event>, …], "next": int}` |
| `GET` | `/import/jobs/{job_id}/report` | — | `text/markdown` |
| `POST` | `/import/jobs/{job_id}/cancel` | — | `{"status": "cancelling"}` |
| `POST` | `/import/jobs/{job_id}/resume` | — | `{"status": "running"}`; only from `interrupted` |
| `POST` | `/import/jobs/{job_id}/rollback` | `{"confirm": "<job_id>"}` | `{"status": "rolling_back"}` |
| `POST` | `/import/jobs/{job_id}/restore-snapshot` | `{"confirm": "<snapshot name>"}` | `{"status": "restoring"}` |

`POST /import/jobs` returns `409 {"error": "busy"}` if another job is running — there
is one global import lock and never two concurrent imports.

Destructive routes require the `confirm` field to equal the value named above; a
mismatch is `400 {"error": "confirm_mismatch"}`. This is the UI's typed confirmation.

### Job object

```jsonc
{
  "job_id": "2026-09-22T10-14-03-ab12",
  "pack_id": "…",
  "mode": "apply",
  "profile": "sop",
  "collection": "sop",
  "status": "running",
  "stage": "upsert",
  "created_at": "2026-09-22T10:14:03Z",
  "started_at": "…", "finished_at": null,
  "operator": "admin",
  "allow_overwrite": false,

  "stages": [
    {"name": "open",      "status": "ok",      "started_at": "…", "finished_at": "…", "detail": "300 points, profile sop"},
    {"name": "contract",  "status": "ok",      "detail": "…"},
    {"name": "probe",     "status": "ok",      "detail": "8 canaries, worst cosine 1.00000"},
    {"name": "preflight", "status": "ok",      "detail": "51 new book_codes, 0 overwrites"},
    {"name": "snapshot",  "status": "ok",      "detail": "sop-2026-09-22-10-14-05.snapshot"},
    {"name": "undo",      "status": "skipped", "detail": "no overwrites"},
    {"name": "upsert",    "status": "running", "detail": "18432/60412"},
    {"name": "indexes",   "status": "pending"},
    {"name": "titles",    "status": "pending"},
    {"name": "verify",    "status": "pending"},
    {"name": "report",    "status": "pending"}
  ],

  "progress": {"points_total": 60412, "points_written": 18432, "batches": 144},
  "snapshot": {"name": "sop-2026-09-22-10-14-05.snapshot", "size": 3821991234},
  "counts": {"created": 0, "overwritten": 0, "books": 51},
  "error": null,
  "rollback": {"available": true, "performed_at": null}
}
```

`status` ∈ `queued · running · ok · failed · cancelled · interrupted · rolled_back`.
Stage `status` ∈ `pending · running · ok · failed · skipped`.

Stage names, in order, are exactly:
`open · contract · probe · preflight · snapshot · undo · upsert · indexes · titles · verify · report`

### Log event (NDJSON, one per line in `log.ndjson`)

```jsonc
{"seq": 41, "ts": "2026-09-22T10:14:31Z", "stage": "upsert",
 "level": "info", "msg": "batch 144/473 ok", "data": {"written": 18432}}
```

`level` ∈ `debug · info · warn · error`. `seq` is monotonic from 0 and is what
`?after=` filters on, so the UI polls without re-reading the whole log.

---

## 4. Job directory on disk

```
/data/jobs/{job_id}/
  job.json                the object above, rewritten atomically on every change
  log.ndjson              append-only
  undo.jsonl              {"id", "payload", "vector"} of every point overwritten
  created_ids.txt         one point id per line — the exact rollback set
  sop_books.before.json   the title table as it was (sop profile only)
  report.md
```

Atomic write = write `job.json.tmp`, `os.replace`. A reader must never see a partial
file.

## 5. UI polling

The page polls `GET /import/jobs/{id}` every 1 s while `status` is `running`, and
`GET /import/jobs/{id}/log?after={next}` alongside it. It stops polling on a terminal
status. No websockets.
