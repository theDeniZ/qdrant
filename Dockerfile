FROM python:3.11-slim

ENV PYTHONUNBUFFERED=1 \
    PIP_NO_CACHE_DIR=1 \
    FASTEMBED_CACHE_PATH=/data/fastembed \
    KEYS_DB=/data/keys.db \
    SOP_BOOKS_JSON=/data/sop_books.json

WORKDIR /srv
COPY requirements.txt .
RUN pip install -r requirements.txt
COPY app ./app
COPY sopack ./sopack
# The embedding contract + calibration fixture (SOPACK-AUTONOMY.md §3.2,
# SOPACK-1.0-PLAN.md §3.6) — single source of truth, shared byte-for-byte
# with the Rust `sopack` binary. Copied to the SAME relative path
# (sopack-rs/contracts, sibling of sopack/) the repo uses, so
# sopack/contract.py's default path resolution (sibling-of-package, no env
# override needed) works unchanged inside the image. No packaged copy under
# sopack/ — one file, one place, never two to keep in sync.
COPY sopack-rs/contracts ./sopack-rs/contracts

# Create persistent storage directories
RUN mkdir -p /data/packs /data/jobs /data/uploads

EXPOSE 8765 8081
HEALTHCHECK --interval=30s --timeout=5s --start-period=20s \
  CMD python -c "import urllib.request; urllib.request.urlopen('http://127.0.0.1:8765/healthz')"
CMD ["python", "-m", "app.server"]
