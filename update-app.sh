#!/usr/bin/env bash
# Rebuild and restart ONLY the Rust app container.
#
# db, redis and vllm keep running untouched — vllm in particular is never
# rebuilt or recreated, so the model stays loaded in GPU memory.
#
# Usage: ./update-app.sh [-p|--pull] [-f|--follow]
set -euo pipefail

cd "$(dirname "$0")"

APP_SERVICE=app
DEPS=(db redis vllm)

PULL=0
FOLLOW=0

usage() {
    cat <<'USAGE'
Usage: ./update-app.sh [options]

Options:
  -p, --pull      git pull --ff-only before building
  -f, --follow    follow the app logs after the restart
  -h, --help      show this help
USAGE
}

while [ $# -gt 0 ]; do
    case "$1" in
        -p|--pull)   PULL=1 ;;
        -f|--follow) FOLLOW=1 ;;
        -h|--help)   usage; exit 0 ;;
        *)           echo "error: unknown option: $1" >&2; usage >&2; exit 2 ;;
    esac
    shift
done

if [ ! -f .env ]; then
    echo "error: .env is missing — create it with: cp .env.dist .env" >&2
    exit 1
fi

if [ "$PULL" -eq 1 ]; then
    echo "==> Pulling latest code"
    git pull --ff-only
fi

echo "==> Checking dependencies"
missing=()
for svc in "${DEPS[@]}"; do
    if [ -z "$(docker compose ps -q --status running "$svc")" ]; then
        missing+=("$svc")
    fi
done
if [ "${#missing[@]}" -gt 0 ]; then
    echo "warning: these services are not running: ${missing[*]}" >&2
    echo "         start them with: docker compose up -d ${missing[*]}" >&2
fi

# Build first: if compilation fails the currently running container is left
# alone, so a broken commit never takes the service down.
echo "==> Building $APP_SERVICE"
docker compose build "$APP_SERVICE"

# --no-deps keeps compose from touching db / redis / vllm.
echo "==> Recreating $APP_SERVICE"
docker compose up -d --no-deps --force-recreate "$APP_SERVICE"

echo "==> Status"
docker compose ps "$APP_SERVICE"

if [ "$FOLLOW" -eq 1 ]; then
    docker compose logs -f "$APP_SERVICE"
else
    docker compose logs --tail=20 "$APP_SERVICE"
fi
