"use strict";
// Corpus import UI — chunked upload, manifest preview, job creation and
// polling. Plain ES2020, no dependencies, no build step. Wire contract:
// docs/IMPORT-API.md.

(() => {
  const PART_SIZE = 8 * 1024 * 1024; // 8 MiB — must match the server (docs/IMPORT-API.md)
  const MAX_RETRIES = 3;
  const RETRY_BASE_MS = 500;
  const POLL_MS = 1000;
  const STAGES = ["open", "contract", "probe", "preflight", "snapshot", "undo",
                  "upsert", "indexes", "verify", "report"];

  // ── small helpers ──────────────────────────────────────────────────────

  function bytesToHex(buf) {
    return Array.from(new Uint8Array(buf)).map((b) => b.toString(16).padStart(2, "0")).join("");
  }

  // Incremental SHA-256, fed the file one slice at a time. Never hash
  // `await file.arrayBuffer()`: materialising a whole 100 MB–1 GB pack as one
  // buffer crashes Chromium-based browsers' browser process (a CHECK in the
  // blob reader — seen with a 130 MB bible pack in Whale). crypto.subtle has
  // no incremental API and exists only in secure contexts anyway (the admin
  // UI is usually plain http on the LAN), so this is pure JS.
  const K256 = new Uint32Array([
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
  ]);

  class Sha256 {
    constructor() {
      this.h = new Uint32Array([0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
                                0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19]);
      this.w = new Uint32Array(64);
      this.tail = new Uint8Array(64); // bytes not yet forming a whole block
      this.tailLen = 0;
      this.len = 0;                   // total bytes fed
    }

    block(src, off) {
      const w = this.w;
      const h = this.h;
      for (let i = 0; i < 16; i += 1) {
        const j = off + i * 4;
        w[i] = ((src[j] << 24) | (src[j + 1] << 16) | (src[j + 2] << 8) | src[j + 3]) >>> 0;
      }
      for (let i = 16; i < 64; i += 1) {
        const x = w[i - 15];
        const y = w[i - 2];
        const s0 = ((x >>> 7) | (x << 25)) ^ ((x >>> 18) | (x << 14)) ^ (x >>> 3);
        const s1 = ((y >>> 17) | (y << 15)) ^ ((y >>> 19) | (y << 13)) ^ (y >>> 10);
        w[i] = (w[i - 16] + s0 + w[i - 7] + s1) >>> 0;
      }
      let a = h[0]; let b = h[1]; let c = h[2]; let d = h[3];
      let e = h[4]; let f = h[5]; let g = h[6]; let hh = h[7];
      for (let i = 0; i < 64; i += 1) {
        const S1 = ((e >>> 6) | (e << 26)) ^ ((e >>> 11) | (e << 21)) ^ ((e >>> 25) | (e << 7));
        const t1 = (hh + S1 + ((e & f) ^ (~e & g)) + K256[i] + w[i]) >>> 0;
        const S0 = ((a >>> 2) | (a << 30)) ^ ((a >>> 13) | (a << 19)) ^ ((a >>> 22) | (a << 10));
        const t2 = (S0 + ((a & b) ^ (a & c) ^ (b & c))) >>> 0;
        hh = g; g = f; f = e; e = (d + t1) >>> 0;
        d = c; c = b; b = a; a = (t1 + t2) >>> 0;
      }
      h[0] += a; h[1] += b; h[2] += c; h[3] += d;
      h[4] += e; h[5] += f; h[6] += g; h[7] += hh;
    }

    update(bytes) {
      let off = 0;
      this.len += bytes.length;
      if (this.tailLen) {
        const take = Math.min(64 - this.tailLen, bytes.length);
        this.tail.set(bytes.subarray(0, take), this.tailLen);
        this.tailLen += take;
        off = take;
        if (this.tailLen < 64) return;
        this.block(this.tail, 0);
        this.tailLen = 0;
      }
      for (; off + 64 <= bytes.length; off += 64) this.block(bytes, off);
      this.tail.set(bytes.subarray(off), 0);
      this.tailLen = bytes.length - off;
    }

    hex() {
      const len = this.len;
      const pad = new Uint8Array(((this.tailLen + 9 + 63) >> 6) << 6);
      pad.set(this.tail.subarray(0, this.tailLen));
      pad[this.tailLen] = 0x80;
      const dv = new DataView(pad.buffer);
      dv.setUint32(pad.length - 8, Math.floor(len / 0x20000000), false); // bit length, high word
      dv.setUint32(pad.length - 4, (len * 8) >>> 0, false);              // bit length, low word
      for (let off = 0; off < pad.length; off += 64) this.block(pad, off);
      const out = new DataView(new ArrayBuffer(32));
      this.h.forEach((v, i) => out.setUint32(i * 4, v, false));
      return bytesToHex(out.buffer);
    }
  }

  async function sha256File(file, onProgress) {
    const hasher = new Sha256();
    for (let start = 0; start < file.size; start += PART_SIZE) {
      const end = Math.min(start + PART_SIZE, file.size);
      // eslint-disable-next-line no-await-in-loop
      hasher.update(new Uint8Array(await file.slice(start, end).arrayBuffer()));
      if (onProgress) onProgress(end, file.size);
    }
    return hasher.hex();
  }

  function fmtBytes(n) {
    if (n === null || n === undefined) return "—";
    const units = ["B", "KB", "MB", "GB", "TB"];
    let i = 0;
    let v = n;
    while (v >= 1024 && i < units.length - 1) {
      v /= 1024;
      i += 1;
    }
    return `${v.toFixed(i > 0 ? 1 : 0)} ${units[i]}`;
  }

  function fmtTime(iso) {
    if (!iso) return "—";
    try {
      return new Date(iso).toLocaleString();
    } catch (e) {
      return String(iso);
    }
  }

  function sleep(ms) {
    return new Promise((resolve) => setTimeout(resolve, ms));
  }

  function el(tag, attrs, children) {
    const node = document.createElement(tag);
    for (const [k, v] of Object.entries(attrs || {})) {
      if (k === "text") node.textContent = v;
      else if (k.startsWith("on") && typeof v === "function") node.addEventListener(k.slice(2), v);
      else node.setAttribute(k, v);
    }
    for (const c of children || []) node.appendChild(c);
    return node;
  }

  async function fetchJSON(url, opts) {
    const res = await fetch(url, opts);
    let body = null;
    try {
      body = await res.json();
    } catch (e) {
      body = null;
    }
    if (!res.ok) {
      const detail = (body && body.detail) || res.statusText || `HTTP ${res.status}`;
      const err = new Error(detail);
      err.code = body && body.error;
      err.status = res.status;
      throw err;
    }
    return body;
  }

  // ── chunked upload ─────────────────────────────────────────────────────

  async function putPart(uploadId, n, blob) {
    let lastErr;
    for (let attempt = 1; attempt <= MAX_RETRIES; attempt += 1) {
      try {
        const res = await fetch(`/import/uploads/${uploadId}/parts/${n}`, {
          method: "PUT",
          headers: { "Content-Type": "application/octet-stream" },
          body: blob,
        });
        if (!res.ok) {
          let detail = `part ${n} failed: HTTP ${res.status}`;
          try {
            const body = await res.json();
            if (body && body.detail) detail = body.detail;
          } catch (e) { /* ignore */ }
          throw new Error(detail);
        }
        return await res.json();
      } catch (err) {
        lastErr = err;
        if (attempt < MAX_RETRIES) await sleep(RETRY_BASE_MS * 2 ** (attempt - 1));
      }
    }
    throw lastErr;
  }

  async function sendParts(uploadId, file, partSize, size, fromPart, onProgress) {
    const totalParts = Math.max(1, Math.ceil(size / partSize));
    let received = fromPart;
    for (let n = fromPart; n < totalParts; n += 1) {
      const start = n * partSize;
      const end = Math.min(start + partSize, size);
      const blob = file.slice(start, end);
      // eslint-disable-next-line no-await-in-loop
      const result = await putPart(uploadId, n, blob);
      received = result.received;
      if (onProgress) onProgress(received, totalParts, end, size);
    }
    return fetchJSON(`/import/uploads/${uploadId}/complete`, { method: "POST" });
  }

  function resumeKey(file) {
    return `sdarm-import-upload:${file.name}:${file.size}`;
  }

  async function uploadFile(file, onProgress, onHashProgress) {
    const key = resumeKey(file);
    const savedId = localStorage.getItem(key);

    if (savedId) {
      try {
        const st = await fetchJSON(`/import/uploads/${savedId}`);
        if (!st.complete) {
          const result = await sendParts(savedId, file, st.part_size, st.size, st.received, onProgress);
          localStorage.removeItem(key);
          return result;
        }
      } catch (err) {
        if (err.status !== 404) throw err;
        // Upload id is gone (pruned) — fall through and start a fresh one.
      }
    }

    const sha256 = await sha256File(file, onHashProgress);
    const created = await fetchJSON("/import/uploads", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ name: file.name, size: file.size, sha256 }),
    });
    try {
      localStorage.setItem(key, created.upload_id);
    } catch (e) { /* private mode etc — resume just won't work, upload still does */ }
    const result = await sendParts(created.upload_id, file, created.part_size, file.size,
                                    created.received || 0, onProgress);
    localStorage.removeItem(key);
    return result;
  }

  // ── page wiring ────────────────────────────────────────────────────────

  document.addEventListener("DOMContentLoaded", () => {
    const dropzone = document.getElementById("dropzone");
    const fileInput = document.getElementById("file-input");
    const progressBar = document.getElementById("upload-bar");
    const progressLabel = document.getElementById("upload-label");
    const manifestBox = document.getElementById("manifest-summary");
    const dryRunBtn = document.getElementById("dry-run-btn");
    const importBtn = document.getElementById("import-btn");
    const overwriteBox = document.getElementById("allow-overwrite");
    const sameTitleBox = document.getElementById("allow-same-title");
    const jobStatus = document.getElementById("job-status");
    const jobBar = document.getElementById("job-bar");
    const logView = document.getElementById("log-view");
    const packsBody = document.querySelector("#packs-table tbody");
    const jobsBody = document.querySelector("#jobs-table tbody");
    const stageList = document.getElementById("stage-list");
    const delLang = document.getElementById("del-lang");
    const delCodes = document.getElementById("del-codes");
    const delLookupBtn = document.getElementById("del-lookup-btn");
    const delPreview = document.getElementById("del-preview");
    const delAllowPaired = document.getElementById("del-allow-paired");
    const delBtn = document.getElementById("del-btn");

    if (!dropzone) return; // not on the import page

    let currentPackId = null;
    let currentJobId = null;
    let logAfter = 0;
    let pollTimer = null;

    // -- manifest -----------------------------------------------------------

    function renderManifest(manifest, packId) {
      const target = manifest.target || {};
      const embedding = manifest.embedding || {};
      const counts = manifest.counts || {};
      manifestBox.innerHTML = "";
      const table = el("table", {}, [
        el("tr", {}, [el("th", { text: "Pack id" }), el("td", { text: packId })]),
        el("tr", {}, [el("th", { text: "Profile" }), el("td", { text: manifest.profile || "—" })]),
        el("tr", {}, [el("th", { text: "Collection" }), el("td", { text: target.collection || "—" })]),
        el("tr", {}, [el("th", { text: "Points" }), el("td", { text: String(counts.points ?? "—") })]),
        el("tr", {}, [el("th", { text: "Books" }), el("td", { text: String(counts.books ?? "—") })]),
        el("tr", {}, [el("th", { text: "Embedding model" }), el("td", { text: embedding.model || "—" })]),
      ]);
      manifestBox.appendChild(el("div", { class: "manifest" }, [table]));
    }

    // -- upload ---------------------------------------------------------------

    async function handleFile(file) {
      manifestBox.textContent = "";
      dryRunBtn.disabled = true;
      importBtn.disabled = true;
      currentPackId = null;
      progressBar.value = 0;
      progressLabel.textContent = `Hashing ${file.name}…`;

      const onProgress = (received, total, bytesDone, bytesTotal) => {
        progressBar.value = Math.round((received / total) * 100);
        progressLabel.textContent =
          `${file.name}: part ${received}/${total} (${fmtBytes(bytesDone)} / ${fmtBytes(bytesTotal)})`;
      };

      try {
        const onHashProgress = (done, total) => {
          progressBar.value = Math.round((done / total) * 100);
          progressLabel.textContent = `Hashing ${file.name}… ${fmtBytes(done)} / ${fmtBytes(total)}`;
        };
        const result = await uploadFile(file, onProgress, onHashProgress);
        progressBar.value = 100;
        progressLabel.textContent = `Uploaded ${file.name} — pack ${result.pack_id}`;
        currentPackId = result.pack_id;
        renderManifest(result.manifest, result.pack_id);
        dryRunBtn.disabled = false;
        importBtn.disabled = false;
        loadPacks();
      } catch (err) {
        progressLabel.textContent = `Upload failed: ${err.message}`;
      }
    }

    dropzone.addEventListener("click", () => fileInput.click());
    dropzone.addEventListener("keydown", (e) => {
      if (e.key === "Enter" || e.key === " ") fileInput.click();
    });
    dropzone.addEventListener("dragover", (e) => {
      e.preventDefault();
      dropzone.classList.add("drag");
    });
    dropzone.addEventListener("dragleave", () => dropzone.classList.remove("drag"));
    dropzone.addEventListener("drop", (e) => {
      e.preventDefault();
      dropzone.classList.remove("drag");
      if (e.dataTransfer.files.length) handleFile(e.dataTransfer.files[0]);
    });
    fileInput.addEventListener("change", () => {
      if (fileInput.files.length) handleFile(fileInput.files[0]);
    });

    // -- job creation + polling ------------------------------------------------

    async function startJob(mode) {
      if (!currentPackId) return;
      dryRunBtn.disabled = true;
      importBtn.disabled = true;
      logView.textContent = "";
      logAfter = 0;
      STAGES.forEach((s) => setStage(s, "pending"));
      jobStatus.innerHTML = "";
      jobStatus.appendChild(el("small", { class: "muted", text: `Starting ${mode}…` }));
      try {
        const created = await fetchJSON("/import/jobs", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({
            pack_id: currentPackId, mode, allow_overwrite: !!overwriteBox.checked,
            allow_same_title: !!(sameTitleBox && sameTitleBox.checked),
          }),
        });
        currentJobId = created.job_id;
        pollJob();
      } catch (err) {
        jobStatus.textContent = `Failed to start job: ${err.message}`;
        dryRunBtn.disabled = false;
        importBtn.disabled = false;
      }
    }

    dryRunBtn.addEventListener("click", () => startJob("dry-run"));
    importBtn.addEventListener("click", () => startJob("apply"));

    function setStage(name, status, detail) {
      const li = document.getElementById(`stage-${name}`);
      if (!li) return;
      li.className = `stage ${status}`;
      li.textContent = detail ? `${name} — ${detail}` : name;
    }

    function renderJobPanel(job) {
      jobStatus.innerHTML = "";
      jobStatus.appendChild(el("b", { text: `${job.mode} — ${job.status}` }));
      if (job.stage) jobStatus.appendChild(el("span", { text: ` (stage: ${job.stage})` }));
      if (job.error) jobStatus.appendChild(el("div", { class: "warn", text: job.error }));

      // An import and a book delete run different stage lists.
      const names = (job.stages || []).map((st) => st.name);
      if (names.join() !== Array.from(stageList.children).map((li) => li.id.slice(6)).join()) {
        stageList.innerHTML = "";
        for (const n of names) stageList.appendChild(el("li", { id: `stage-${n}`, class: "stage pending", text: n }));
      }
      for (const st of job.stages || []) setStage(st.name, st.status, st.detail);

      const prog = job.progress || {};
      if (prog.points_total) {
        jobBar.max = prog.points_total;
        jobBar.value = prog.points_written || 0;
      } else {
        jobBar.value = job.status === "ok" ? 100 : 0;
        jobBar.max = 100;
      }
    }

    async function pollLog() {
      try {
        const { events, next } = await fetchJSON(`/import/jobs/${currentJobId}/log?after=${logAfter}`);
        logAfter = next;
        for (const ev of events) {
          const line = `[${ev.ts}] ${ev.stage || "-"} ${ev.level}: ${ev.msg}\n`;
          logView.textContent += line;
        }
        logView.scrollTop = logView.scrollHeight;
      } catch (err) { /* transient — next poll will retry */ }
    }

    const TERMINAL = new Set(["ok", "failed", "cancelled", "interrupted", "rolled_back"]);

    async function pollJob() {
      if (pollTimer) clearTimeout(pollTimer);
      if (!currentJobId) return;
      try {
        const job = await fetchJSON(`/import/jobs/${currentJobId}`);
        renderJobPanel(job);
        await pollLog();
        if (TERMINAL.has(job.status)) {
          dryRunBtn.disabled = !currentPackId;
          importBtn.disabled = !currentPackId;
          loadJobs();
          return;
        }
      } catch (err) {
        jobStatus.textContent = `Polling failed: ${err.message}`;
      }
      pollTimer = setTimeout(pollJob, POLL_MS);
    }

    // -- book delete --------------------------------------------------------------

    let delBooks = [];

    function delLabel(b) { return `${b.lang}:${b.book_code}`; }

    async function lookupBooks() {
      delBtn.disabled = true;
      delBooks = [];
      delPreview.innerHTML = "";
      const lang = delLang.value.trim();
      const codes = delCodes.value.split(",").map((c) => c.trim()).filter(Boolean);
      if (!lang || !codes.length) return;
      let data;
      try {
        data = await fetchJSON(`/import/books?lang=${encodeURIComponent(lang)}&codes=${encodeURIComponent(codes.join(","))}`);
      } catch (err) {
        delPreview.textContent = `Lookup failed: ${err.message}`;
        return;
      }
      const rows = data.books.map((b) => el("tr", {}, [
        el("td", {}, [el("code", { text: delLabel(b) })]),
        el("td", { text: b.title || "—" }),
        el("td", { text: b.author || "—" }),
        el("td", { text: String(b.year ?? "—") }),
        el("td", { text: b.corpus || "—" }),
        el("td", { class: b.points ? "" : "warn", text: b.points ? String(b.points) : "not found" }),
        el("td", { class: b.paired.length ? "warn" : "", text: b.paired.join(", ") || "—" }),
      ]));
      delPreview.appendChild(el("div", { class: "manifest" }, [el("table", {}, [
        el("tr", {}, ["Book", "Title", "Author", "Year", "Corpus", "Points", "Paired translations"]
          .map((h) => el("th", { text: h }))),
        ...rows,
      ])]));
      delBooks = data.books.filter((b) => b.points).map((b) => ({ lang: b.lang, book_code: b.book_code }));
      delBtn.disabled = !delBooks.length || delBooks.length !== data.books.length;
    }

    async function deleteBooks() {
      const want = delBooks.map(delLabel).join(",");
      const typed = prompt(`Type the books to confirm the delete:\n${want}`);
      if (typed !== want) { if (typed !== null) alert("Did not match — nothing deleted."); return; }
      delBtn.disabled = true;
      logView.textContent = "";
      logAfter = 0;
      try {
        const created = await fetchJSON("/import/deletes", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ books: delBooks, confirm: typed, allow_paired: !!delAllowPaired.checked }),
        });
        currentJobId = created.job_id;
        pollJob();
      } catch (err) {
        jobStatus.textContent = `Failed to start delete: ${err.message}`;
        delBtn.disabled = false;
      }
    }

    delLookupBtn.addEventListener("click", lookupBooks);
    delCodes.addEventListener("input", () => { delBtn.disabled = true; });
    delLang.addEventListener("input", () => { delBtn.disabled = true; });
    delBtn.addEventListener("click", deleteBooks);

    // -- packs table ------------------------------------------------------------

    async function loadPacks() {
      packsBody.innerHTML = "";
      let data;
      try {
        data = await fetchJSON("/import/packs");
      } catch (err) {
        packsBody.appendChild(el("tr", {}, [el("td", { colspan: "9", text: err.message })]));
        return;
      }
      if (!data.packs.length) {
        packsBody.appendChild(el("tr", {}, [el("td", { colspan: "9", text: "No packs uploaded yet." })]));
        return;
      }
      for (const p of data.packs) {
        const delBtn = el("button", {
          text: "Delete",
          onclick: async () => {
            if (!confirm(`Delete pack ${p.pack_id}? This does not touch anything already imported.`)) return;
            try {
              await fetchJSON(`/import/packs/${p.pack_id}`, { method: "DELETE" });
              loadPacks();
            } catch (err) {
              alert(`Delete failed: ${err.message}`);
            }
          },
        });
        packsBody.appendChild(el("tr", {}, [
          el("td", {}, [el("code", { text: p.pack_id })]),
          el("td", { text: p.name || "—" }),
          el("td", { text: p.profile || "—" }),
          el("td", { text: String(p.points ?? "—") }),
          el("td", { text: String(p.books ?? "—") }),
          el("td", { text: fmtBytes(p.bytes) }),
          el("td", { text: fmtTime(p.uploaded_at ? p.uploaded_at * 1000 : null) }),
          el("td", { text: (p.imported_by || []).join(", ") || "—" }),
          el("td", {}, [delBtn]),
        ]));
      }
    }

    // -- jobs table ---------------------------------------------------------------

    function jobActionButton(label, handler) {
      return el("button", { text: label, onclick: handler });
    }

    async function postJobAction(jobId, action, body) {
      return fetchJSON(`/import/jobs/${jobId}/${action}`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body || {}),
      });
    }

    async function loadJobs() {
      jobsBody.innerHTML = "";
      let data;
      try {
        data = await fetchJSON("/import/jobs");
      } catch (err) {
        jobsBody.appendChild(el("tr", {}, [el("td", { colspan: "8", text: err.message })]));
        return;
      }
      if (!data.jobs.length) {
        jobsBody.appendChild(el("tr", {}, [el("td", { colspan: "8", text: "No jobs yet." })]));
        return;
      }
      for (const j of data.jobs) {
        const actions = [];
        actions.push(el("a", { href: `/import/jobs/${j.job_id}/report`, target: "_blank", text: "report" }));

        if (j.status === "running") {
          actions.push(jobActionButton("Cancel", async () => {
            try { await postJobAction(j.job_id, "cancel"); loadJobs(); } catch (e) { alert(e.message); }
          }));
        }
        if (j.status === "interrupted") {
          actions.push(jobActionButton("Resume", async () => {
            try { await postJobAction(j.job_id, "resume"); loadJobs(); } catch (e) { alert(e.message); }
          }));
        }
        if (j.rollback && j.rollback.available && !j.rollback.performed_at) {
          actions.push(jobActionButton("Rollback", async () => {
            const typed = prompt(`Type the job id to confirm rollback:\n${j.job_id}`);
            if (typed !== j.job_id) { if (typed !== null) alert("Job id did not match — not rolled back."); return; }
            try { await postJobAction(j.job_id, "rollback", { confirm: typed }); loadJobs(); }
            catch (e) { alert(e.message); }
          }));
        }
        if (j.snapshot && j.snapshot.name) {
          actions.push(jobActionButton("Restore snapshot", async () => {
            const typed = prompt(`Type the snapshot name to confirm restore:\n${j.snapshot.name}`);
            if (typed !== j.snapshot.name) { if (typed !== null) alert("Snapshot name did not match — not restored."); return; }
            try { await postJobAction(j.job_id, "restore-snapshot", { confirm: typed }); loadJobs(); }
            catch (e) { alert(e.message); }
          }));
        }
        actions.push(jobActionButton("View", () => {
          currentJobId = j.job_id;
          logAfter = 0;
          logView.textContent = "";
          pollJob();
        }));

        const actionsCell = el("td", {}, actions);
        jobsBody.appendChild(el("tr", {}, [
          el("td", {}, [el("code", { text: j.job_id })]),
          el("td", {}, [el("code", { text: j.kind === "delete"
            ? (j.books || []).map(delLabel).join(", ") : j.pack_id })]),
          el("td", { text: j.kind === "delete" ? "delete" : j.mode }),
          el("td", { text: j.status }),
          el("td", { text: j.stage || "—" }),
          el("td", { text: fmtTime(j.created_at) }),
          el("td", { text: j.operator || "—" }),
          actionsCell,
        ]));
      }
    }

    loadPacks();
    loadJobs();
  });
})();
