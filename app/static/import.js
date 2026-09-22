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
                  "upsert", "indexes", "titles", "verify", "report"];

  // ── small helpers ──────────────────────────────────────────────────────

  function bytesToHex(buf) {
    return Array.from(new Uint8Array(buf)).map((b) => b.toString(16).padStart(2, "0")).join("");
  }

  async function sha256File(file) {
    const buf = await file.arrayBuffer();
    const digest = await crypto.subtle.digest("SHA-256", buf);
    return bytesToHex(digest);
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

  async function uploadFile(file, onProgress) {
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

    const sha256 = await sha256File(file);
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
    const jobStatus = document.getElementById("job-status");
    const jobBar = document.getElementById("job-bar");
    const logView = document.getElementById("log-view");
    const packsBody = document.querySelector("#packs-table tbody");
    const jobsBody = document.querySelector("#jobs-table tbody");

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
        const result = await uploadFile(file, onProgress);
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
          el("td", {}, [el("code", { text: j.pack_id })]),
          el("td", { text: j.mode }),
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
