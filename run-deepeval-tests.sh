#!/usr/bin/env bash
# Run the DeepEval suites in evals/ against the running app and write the report.
#
# Creates evals/.venv on the first run (Python 3.11) and reinstalls the requirements
# when evals/requirements.txt changes. Every argument goes to evals/run.py.
#
# Usage: ./run-deepeval-tests.sh                       every suite
#        ./run-deepeval-tests.sh -r 3                  every golden three times
#        ./run-deepeval-tests.sh test_form.py -k de    one suite, plus any pytest options
#        ./run-deepeval-tests.sh --report              the report of the latest run again
set -euo pipefail

cd "$(dirname "$0")"

EVALS=evals
VENV="$EVALS/.venv"
REQUIREMENTS="$EVALS/requirements.txt"
STAMP="$VENV/.requirements.sha"
APP_URL="${EVAL_APP_URL:-http://localhost:8080}"

usage() {
    sed -n '2,10p' "$0" | sed 's/^# \{0,1\}//'
}

find_python() {
    if command -v python3.11 >/dev/null 2>&1; then
        command -v python3.11
        return
    fi
    for candidate in "$HOME"/.pyenv/versions/3.11*/bin/python; do
        if [ -x "$candidate" ]; then
            echo "$candidate"
            return
        fi
    done
    echo "warning: Python 3.11 not found, using python3 (the suites were made with 3.11)" >&2
    command -v python3
}

if [ "${1:-}" = "-h" ] || [ "${1:-}" = "--help" ]; then
    usage
    exit 0
fi

# ── Virtual environment ────────────────────────────────────────────────────
if [ ! -x "$VENV/bin/python" ]; then
    echo "==> Creating $VENV"
    "$(find_python)" -m venv "$VENV"
fi

wanted="$(shasum "$REQUIREMENTS" | cut -d' ' -f1)"
if [ "$(cat "$STAMP" 2>/dev/null)" != "$wanted" ]; then
    echo "==> Installing $REQUIREMENTS"
    "$VENV/bin/python" -m pip install -q --disable-pip-version-check -r "$REQUIREMENTS"
    echo "$wanted" > "$STAMP"
fi

if [ "${1:-}" = "--report" ]; then
    exec "$VENV/bin/python" "$EVALS/report.py"
fi

# ── The app and the model ──────────────────────────────────────────────────
if ! curl -fsS -m 5 "$APP_URL/version" >/dev/null; then
    echo "error: the app at $APP_URL does not answer - start it with ./update.sh" >&2
    exit 1
fi

# The report names the model it was made with. On the Mac the model is served by
# Ollama, not by vLLM on port 8000.
if [ -z "${EVAL_LLM_URL:-}" ] && curl -fsS -m 2 http://localhost:11434/api/version >/dev/null 2>&1; then
    export EVAL_LLM_URL=http://localhost:11434/v1
fi

echo "==> Running the suites against $APP_URL (model server: ${EVAL_LLM_URL:-http://localhost:8000/v1})"
exec "$VENV/bin/python" "$EVALS/run.py" "$@"
