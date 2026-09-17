#!/usr/bin/env bash
# Translate LLM_* environment variables into `vllm serve` CLI flags.
#
# Note the LLM_ prefix rather than VLLM_: vLLM reserves the VLLM_ namespace for
# its own settings (VLLM_PORT, VLLM_API_KEY, ...) and warns about — or worse,
# acts on — anything else it finds there.
set -euo pipefail

args=(
  --model "${LLM_MODEL}"
  --host "${LLM_HOST}"
  --port "${LLM_PORT}"
  --max-model-len "${LLM_MAX_MODEL_LEN}"
  --gpu-memory-utilization "${LLM_GPU_MEMORY_UTILIZATION}"
)

case "$(printf '%s' "${LLM_ENABLE_PREFIX_CACHING:-}" | tr '[:upper:]' '[:lower:]')" in
  1|true|yes|on) args+=(--enable-prefix-caching) ;;
  *)             args+=(--no-enable-prefix-caching) ;;
esac

if [ -n "${LLM_SERVED_MODEL_NAME:-}" ]; then
  args+=(--served-model-name "${LLM_SERVED_MODEL_NAME}")
fi

if [ -n "${LLM_API_KEY:-}" ]; then
  args+=(--api-key "${LLM_API_KEY}")
fi

if [ -n "${LLM_TENSOR_PARALLEL_SIZE:-}" ]; then
  args+=(--tensor-parallel-size "${LLM_TENSOR_PARALLEL_SIZE}")
fi

if [ -n "${LLM_DTYPE:-}" ]; then
  args+=(--dtype "${LLM_DTYPE}")
fi

# Free-form escape hatch for anything not covered above, e.g.
# LLM_EXTRA_ARGS="--quantization awq --swap-space 8"
if [ -n "${LLM_EXTRA_ARGS:-}" ]; then
  # shellcheck disable=SC2206 # intentional word splitting
  args+=(${LLM_EXTRA_ARGS})
fi

echo "Starting: vllm serve ${args[*]}" >&2
exec vllm serve "${args[@]}" "$@"
