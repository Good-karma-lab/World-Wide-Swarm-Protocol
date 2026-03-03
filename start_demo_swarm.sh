#!/bin/bash
# Start 30 local wws-connector nodes for demo.
#
# P2P ports:  9700-9729  (one per node)
# RPC ports:  9730,9732,...9788  (even)
# HTTP ports: 9731,9733,...9789  (odd)
#
# Usage: ./start_demo_swarm.sh [path/to/wws-connector]

BIN="${1:-$(dirname "$0")/wws-connector}"
LOGDIR="/tmp/wws-demo-swarm"
mkdir -p "$LOGDIR"

# Kill any previous demo swarm
pkill -9 -f "wws-connector" 2>/dev/null
sleep 2

echo "Starting demo node-1 (bootstrap)..."
"$BIN" \
  --agent-name demo-1 \
  --listen /ip4/0.0.0.0/tcp/9700 \
  --rpc 127.0.0.1:9730 \
  --files-addr 127.0.0.1:9731 \
  > "$LOGDIR/node-1.log" 2>&1 &
PID1=$!
echo "  node-1 pid=$PID1 RPC=9730 HTTP=9731"

# Wait for node 1 to be ready and get peer ID
echo "Waiting for node-1..."
LOCAL_PEER_ID=""
for i in $(seq 1 20); do
    sleep 1
    DID=$(python3 -c "
import socket, json
try:
    req = json.dumps({'jsonrpc':'2.0','id':'1','method':'swarm.get_status','params':{},'signature':''}) + '\n'
    s = socket.socket(); s.settimeout(2)
    s.connect(('127.0.0.1', 9730))
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
        echo "  node-1 ready: peer=$LOCAL_PEER_ID"
        break
    fi
done

if [ -z "$LOCAL_PEER_ID" ]; then
    echo "ERROR: node-1 failed to start. Check $LOGDIR/node-1.log"
    exit 1
fi

LOCAL_BOOTSTRAP="/ip4/127.0.0.1/tcp/9700/p2p/$LOCAL_PEER_ID"

# Start nodes 2-30
echo ""
echo "Starting nodes 2-30..."
for i in $(seq 2 30); do
    P2P_PORT=$((9699 + i))
    RPC_PORT=$((9728 + i * 2))
    HTTP_PORT=$((9729 + i * 2))

    "$BIN" \
        --agent-name "demo-$i" \
        --listen "/ip4/0.0.0.0/tcp/$P2P_PORT" \
        --rpc "127.0.0.1:$RPC_PORT" \
        --files-addr "127.0.0.1:$HTTP_PORT" \
        --bootstrap "$LOCAL_BOOTSTRAP" \
        > "$LOGDIR/node-$i.log" 2>&1 &
    echo "  node-$i pid=$! P2P=$P2P_PORT RPC=$RPC_PORT HTTP=$HTTP_PORT"
done

echo ""
echo "All 30 demo nodes started."
echo "Logs:    $LOGDIR/"
echo "Node 1:  RPC 127.0.0.1:9730  HTTP 127.0.0.1:9731"
echo "Web UI:  http://127.0.0.1:9731"
