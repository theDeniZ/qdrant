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

# Create persistent storage directories
RUN mkdir -p /data/packs /data/jobs /data/uploads

EXPOSE 8765 8081
HEALTHCHECK --interval=30s --timeout=5s --start-period=20s \
  CMD python -c "import urllib.request; urllib.request.urlopen('http://127.0.0.1:8765/healthz')"
CMD ["python", "-m", "app.server"]
