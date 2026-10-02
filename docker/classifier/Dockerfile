# ── Classifier: machine #1 without an LLM, on the CPU ─────────────────────
# Serves one GLiNER2 model over HTTP (server.py). Only used with
# INTENT_MATCHER=classifier; the GPU stays vLLM's.
FROM python:3.12-slim

WORKDIR /classifier

# The CPU build of torch: a fraction of the CUDA one's size
RUN pip install --no-cache-dir torch==2.14.0 --index-url https://download.pytorch.org/whl/cpu

COPY requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt

COPY server.py .

# Models are cached here; mount a host volume to survive container rebuilds
ENV HF_HOME=/root/.cache/huggingface

EXPOSE 8090

CMD ["uvicorn", "server:app", "--host", "0.0.0.0", "--port", "8090"]
