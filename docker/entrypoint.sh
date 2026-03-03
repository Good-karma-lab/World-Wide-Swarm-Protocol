#!/bin/sh
# Dynamic bootstrap resolver for WWS nodes.
# If --bootstrap flag contains "RESOLVE:<host>:<port>", query that node's
# /api/identity to get the real peer ID before starting.
set -e

ARGS=""
BOOTSTRAP_HOST=""
BOOTSTRAP_P2P_PORT=""

for arg in "$@"; do
  case "$arg" in
    RESOLVE:*:*)
      # Format: RESOLVE:<http-host>:<http-port>:<p2p-port>
      BOOTSTRAP_HOST=$(echo "$arg" | cut -d: -f2)
      HTTP_PORT=$(echo "$arg" | cut -d: -f3)
      BOOTSTRAP_P2P_PORT=$(echo "$arg" | cut -d: -f4)
      ;;
    *)
      ARGS="$ARGS $arg"
      ;;
  esac
done

if [ -n "$BOOTSTRAP_HOST" ]; then
  echo "[entrypoint] Waiting for bootstrap node $BOOTSTRAP_HOST:$HTTP_PORT ..."
  until wget -qO- "http://$BOOTSTRAP_HOST:$HTTP_PORT/api/health" > /dev/null 2>&1; do
    sleep 1
  done

  PEER_ID=$(wget -qO- "http://$BOOTSTRAP_HOST:$HTTP_PORT/api/identity" | \
    python3 -c "import json,sys; d=json.load(sys.stdin); print(d.get('peer_id', ''))")

  if [ -z "$PEER_ID" ]; then
    echo "[entrypoint] Failed to get peer ID from $BOOTSTRAP_HOST:$HTTP_PORT" >&2
    exit 1
  fi

  BOOTSTRAP_ADDR="/ip4/$BOOTSTRAP_HOST/tcp/$BOOTSTRAP_P2P_PORT/p2p/$PEER_ID"
  echo "[entrypoint] Bootstrap resolved: $BOOTSTRAP_ADDR"
  ARGS="$ARGS --bootstrap $BOOTSTRAP_ADDR"
fi

exec wws-connector $ARGS
