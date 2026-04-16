#!/usr/bin/env bash
# Idempotent production deploy script for the Wellex Rust API.
#
# Usage (from your laptop):
#   VPS_HOST=root@100.90.71.111 ./deploy.sh
#
# Or on the VPS directly:
#   ssh root@100.90.71.111
#   cd ~/wvi-api-rust && ./deploy.sh --local
#
# Requires on the VPS:
#   - git
#   - docker + docker compose
#   - the `claude` CLI in PATH + ~/.claude/ authenticated (for Sonnet 4.6)
#   - the repo cloned at $REPO_PATH (default ~/wvi-api-rust)

set -euo pipefail

REPO_PATH="${REPO_PATH:-$HOME/wvi-api-rust}"
VPS_HOST="${VPS_HOST:-root@100.90.71.111}"

run_local() {
    echo "=== [1/5] git pull origin master ==="
    cd "$REPO_PATH"
    git fetch origin
    git reset --hard origin/master

    echo ""
    echo "=== [2/5] rebuild API container ==="
    docker compose build api

    echo ""
    echo "=== [3/5] restart API (zero-downtime via rolling up) ==="
    docker compose up -d --no-deps api

    echo ""
    echo "=== [4/5] wait for health ==="
    for i in 1 2 3 4 5 6 7 8 9 10; do
        if curl -sf http://localhost:8091/api/v1/health/server-status > /dev/null 2>&1; then
            echo "API healthy after ${i}s"
            break
        fi
        sleep 1
    done

    echo ""
    echo "=== [5/5] smoke test new AI endpoints ==="
    for ep in daily-brief evening-review anomaly-alert weekly-deep full-analysis ecg-interpret recovery-deep; do
        code=$(curl -s -o /dev/null -w "%{http_code}" -X POST \
            -H "Authorization: Bearer dev-token" \
            -H "Content-Type: application/json" \
            -d '{}' \
            "http://localhost:8091/api/v1/ai/$ep")
        echo "  $code /api/v1/ai/$ep"
    done

    echo ""
    echo "=== Deploy complete ==="
    docker compose ps
}

run_remote() {
    echo "=== Deploying to $VPS_HOST ==="
    ssh -o StrictHostKeyChecking=accept-new "$VPS_HOST" "cd $REPO_PATH && ./deploy.sh --local"
}

case "${1:-remote}" in
    --local)
        run_local
        ;;
    *)
        run_remote
        ;;
esac
