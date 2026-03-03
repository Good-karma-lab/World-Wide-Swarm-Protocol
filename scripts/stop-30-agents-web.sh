#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PID_FILE="/tmp/wws-swarm/operator-web-30.pid"

cd "$ROOT_DIR"

if [ -f "$PID_FILE" ]; then
    PID="$(cat "$PID_FILE" 2>/dev/null || true)"
    if [ -n "${PID:-}" ] && kill -0 "$PID" 2>/dev/null; then
        echo "[stop-30] Stopping dedicated web console (PID: $PID)"
        kill "$PID" >/dev/null 2>&1 || true
        wait "$PID" 2>/dev/null || true
    fi
    rm -f "$PID_FILE"
fi

echo "[stop-30] Stopping swarm nodes and agents"
./swarm-manager.sh stop

echo "[stop-30] Done"
