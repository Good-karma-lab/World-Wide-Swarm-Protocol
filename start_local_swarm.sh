#!/bin/bash
# Start 20 local wws-connector nodes that join the same public swarm as Docker nodes.
#
# P2P ports:  9500-9519  (one per node)
# RPC ports:  9520,9522,...9558  (even)
# HTTP ports: 9521,9523,...9559  (odd)
#
# Docker node-1 P2P bootstrap: 127.0.0.1:9900 (mapped from container)
# Swarm ID: "public" (same as Docker nodes)

BIN="$(dirname "$0")/target/release/wws-connector"
LOGDIR="/tmp/wws-local-swarm"
mkdir -p "$LOGDIR"

# Kill any previous local swarm (host processes only; Docker has its own)
pkill -9 -f "wws-connector" 2>/dev/null
sleep 2

# Docker node-1 bootstrap: wait for it to be ready and get its peer ID
echo "Getting Docker node-1 peer ID (P2P port 9900)..."
DOCKER_PEER_ID=""
for i in $(seq 1 15); do
    DID=$(python3 -c "
import socket, json
try:
    req = json.dumps({'jsonrpc':'2.0','id':'1','method':'swarm.get_status','params':{},'signature':''}) + '\n'
    s = socket.socket(); s.settimeout(2)
    s.connect(('127.0.0.1', 9370))
    s.sendall(req.encode()); s.shutdown(1)
    data = b''
    while True:
        c = s.recv(4096)
        if not c: break
        data += c
    s.close()
    print(json.loads(data).get('result',{}).get('agent_id',''))
except: print('')
" 2>/dev/null)
    if [ -n "$DID" ]; then
        DOCKER_PEER_ID="${DID#did:swarm:}"
        echo "  Docker node-1 peer: $DOCKER_PEER_ID"
        break
    fi
    sleep 1
done

DOCKER_BOOTSTRAP=""
if [ -n "$DOCKER_PEER_ID" ]; then
    DOCKER_BOOTSTRAP="/ip4/127.0.0.1/tcp/9900/p2p/$DOCKER_PEER_ID"
    echo "  Bootstrap: $DOCKER_BOOTSTRAP"
else
    echo "  WARNING: Docker node-1 not reachable, starting local-only swarm"
fi
echo ""

echo "Starting local node-1..."
"$BIN" \
  --agent-name local-1 \
  --listen /ip4/0.0.0.0/tcp/9500 \
  --rpc 127.0.0.1:9520 \
  --files-addr 127.0.0.1:9521 \
  ${DOCKER_BOOTSTRAP:+--bootstrap "$DOCKER_BOOTSTRAP"} \
  > "$LOGDIR/node-1.log" 2>&1 &
PID1=$!
echo "  local node-1 pid=$PID1 RPC=9520 HTTP=9521"

# Wait for local node-1 to be ready and get peer ID
echo "Waiting for local node-1..."
LOCAL_PEER_ID=""
for i in $(seq 1 20); do
    sleep 1
    DID=$(python3 -c "
import socket, json
try:
    req = json.dumps({'jsonrpc':'2.0','id':'1','method':'swarm.get_status','params':{},'signature':''}) + '\n'
    s = socket.socket(); s.settimeout(2)
    s.connect(('127.0.0.1', 9520))
    s.sendall(req.encode()); s.shutdown(1)
    data = b''
    while True:
        c = s.recv(4096)
        if not c: break
        data += c
    s.close()
    print(json.loads(data).get('result',{}).get('agent_id',''))
except: print('')
" 2>/dev/null)
    if [ -n "$DID" ]; then
        LOCAL_PEER_ID="${DID#did:swarm:}"
        echo "  local node-1 ready: $LOCAL_PEER_ID"
        break
    fi
done

if [ -z "$LOCAL_PEER_ID" ]; then
    echo "ERROR: local node-1 failed to start"
    exit 1
fi

LOCAL_BOOTSTRAP="/ip4/127.0.0.1/tcp/9500/p2p/$LOCAL_PEER_ID"

# Start nodes 2-20: bootstrap from BOTH local node-1 AND Docker node-1
echo ""
echo "Starting local nodes 2-20..."
for i in $(seq 2 20); do
    P2P_PORT=$((9499 + i))
    RPC_PORT=$((9518 + i*2))
    HTTP_PORT=$((9519 + i*2))

    BOOTSTRAP_ARGS="--bootstrap $LOCAL_BOOTSTRAP"
    if [ -n "$DOCKER_BOOTSTRAP" ]; then
        BOOTSTRAP_ARGS="$BOOTSTRAP_ARGS --bootstrap $DOCKER_BOOTSTRAP"
    fi

    "$BIN" \
        --agent-name "local-$i" \
        --listen "/ip4/0.0.0.0/tcp/$P2P_PORT" \
        --rpc "127.0.0.1:$RPC_PORT" \
        --files-addr "127.0.0.1:$HTTP_PORT" \
        $BOOTSTRAP_ARGS \
        > "$LOGDIR/node-$i.log" 2>&1 &
    echo "  node-$i pid=$! P2P=$P2P_PORT RPC=$RPC_PORT HTTP=$HTTP_PORT"
done

echo ""
echo "All 20 local nodes started. Logs: $LOGDIR/"
echo "Both local and Docker swarms now share swarm_id=public"
echo "Local node-1:  RPC 127.0.0.1:9520  HTTP 127.0.0.1:9521"
echo "Docker node-1: RPC 127.0.0.1:9370  HTTP 127.0.0.1:9371"
