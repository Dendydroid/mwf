# ── vLLM OpenAI-compatible inference server ───────────────────────────────
# Thin wrapper around the official image so every runtime knob comes from
# environment variables (see .env.dist) instead of hardcoded CLI flags.
ARG VLLM_IMAGE=vllm/vllm-openai:latest
FROM ${VLLM_IMAGE}

# curl is only needed for the container healthcheck
RUN apt-get update && apt-get install -y --no-install-recommends \
    curl \
    && rm -rf /var/lib/apt/lists/*

COPY entrypoint.sh /usr/local/bin/entrypoint.sh
RUN chmod +x /usr/local/bin/entrypoint.sh

# Models are cached here; mount a host volume to survive container rebuilds
ENV HF_HOME=/root/.cache/huggingface

# Our own knobs are LLM_*, not VLLM_*, which is vLLM's reserved namespace.
ENV LLM_MODEL=Qwen/Qwen2.5-7B-Instruct-AWQ \
    LLM_HOST=0.0.0.0 \
    LLM_PORT=8000 \
    LLM_MAX_MODEL_LEN=4096 \
    LLM_GPU_MEMORY_UTILIZATION=0.90 \
    LLM_ENABLE_PREFIX_CACHING=true \
    LLM_EXTRA_ARGS=""

EXPOSE 8000

# The base image sets its own ENTRYPOINT; override it with ours.
ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
