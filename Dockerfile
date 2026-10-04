FROM python:3.11-slim

ENV PYTHONUNBUFFERED=1 \
    PIP_NO_CACHE_DIR=1 \
    FASTEMBED_CACHE_PATH=/data/fastembed \
    KEYS_DB=/data/keys.db

WORKDIR /srv
COPY requirements.txt .
RUN pip install -r requirements.txt
COPY app ./app
# The embedding contract + calibration fixture — single source of truth,
# shared byte-for-byte with the Rust `sopack` binary that writes packs.
# Copied to the SAME relative path the repo uses (sopack-rs/contracts, sibling
# of app/), so app/pack/contract.py resolves it without an env override.
COPY sopack-rs/contracts ./sopack-rs/contracts

# The volume holds only what the server needs to run: keys.db, the fastembed
# model cache and the import pipeline's working dirs. Corpus data and all of
# its metadata live in Qdrant.
RUN mkdir -p /data/packs /data/jobs /data/uploads

EXPOSE 8765 8081
HEALTHCHECK --interval=30s --timeout=5s --start-period=20s \
  CMD python -c "import urllib.request; urllib.request.urlopen('http://127.0.0.1:8765/healthz')"
CMD ["python", "-m", "app.server"]
