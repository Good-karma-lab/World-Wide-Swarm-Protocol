#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Ensure cargo is on PATH in non-interactive shells.
export PATH="$HOME/.cargo/bin:$PATH"

if [ -f "$ROOT_DIR/scripts/load-env.sh" ]; then
    # shellcheck disable=SC1091
    source "$ROOT_DIR/scripts/load-env.sh"
fi

cd "$ROOT_DIR"

AGENT_COUNT=30
CONSOLE_P2P_PORT=22900
CONSOLE_RPC_PORT=22970
CONSOLE_WEB_PORT=22971

echo "[run-30] Ensuring standalone web app is built"
if [ ! -d "$ROOT_DIR/webapp/node_modules" ]; then
    (cd "$ROOT_DIR/webapp" && npm install)
fi
(cd "$ROOT_DIR/webapp" && npm run build >/dev/null)

echo "[run-30] Building connector"
if ! command -v cargo >/dev/null 2>&1; then
    echo "[run-30] cargo not found in PATH. Install Rust or add cargo to PATH."
    exit 1
fi
cargo build --release -p wws-connector >/dev/null

echo "[run-30] Starting $AGENT_COUNT agents"
echo "[run-30] LLM backend: ${LLM_BACKEND:-unset}, model: ${MODEL_NAME:-unset}"
./swarm-manager.sh start-agents "$AGENT_COUNT"

RPC_PORT=$(awk -F'|' 'NR==1 {print $5}' /tmp/wws-swarm/nodes.txt)
P2P_PORT=$(awk -F'|' 'NR==1 {print $4}' /tmp/wws-swarm/nodes.txt)

if [ -z "$RPC_PORT" ] || [ -z "$P2P_PORT" ]; then
    echo "[run-30] Failed to read bootstrap ports from /tmp/wws-swarm/nodes.txt"
    exit 1
fi

PEER_ID=$(echo '{"jsonrpc":"2.0","method":"swarm.get_status","params":{},"id":"status","signature":""}' | nc 127.0.0.1 "$RPC_PORT" | python3 -c 'import json,sys; d=json.load(sys.stdin); print(d["result"]["agent_id"].replace("did:swarm:",""))')
BOOTSTRAP_ADDR="/ip4/127.0.0.1/tcp/$P2P_PORT/p2p/$PEER_ID"

echo "[run-30] Starting dedicated web console connector"
./target/release/wws-connector \
  --listen "/ip4/127.0.0.1/tcp/$CONSOLE_P2P_PORT" \
  --rpc "127.0.0.1:$CONSOLE_RPC_PORT" \
  --files-addr "127.0.0.1:$CONSOLE_WEB_PORT" \
  --bootstrap "$BOOTSTRAP_ADDR" \
  --agent-name "operator-web-30" \
  > /tmp/wws-swarm/operator-web-30.log 2>&1 &

CONSOLE_PID=$!
echo "$CONSOLE_PID" > /tmp/wws-swarm/operator-web-30.pid

for _ in $(seq 1 80); do
    if curl -sf "http://127.0.0.1:$CONSOLE_WEB_PORT/api/health" >/dev/null; then
        break
    fi
    sleep 1
done

URL="http://127.0.0.1:$CONSOLE_WEB_PORT/"
echo "[run-30] Web console ready: $URL"
echo "[run-30] Console log: /tmp/wws-swarm/operator-web-30.log"
echo "[run-30] Stop all nodes: ./swarm-manager.sh stop"

if command -v open >/dev/null 2>&1; then
    open "$URL" || true
elif command -v xdg-open >/dev/null 2>&1; then
    xdg-open "$URL" || true
else
    echo "[run-30] No browser opener found; open this URL manually: $URL"
fi
