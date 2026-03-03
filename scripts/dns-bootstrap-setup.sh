#!/usr/bin/env bash
# dns-bootstrap-setup.sh — Generate DNS TXT records for WWS bootstrap discovery.
#
# Usage:
#   ./scripts/dns-bootstrap-setup.sh [RPC_ADDR] [DOMAIN]
#
# Queries the local wws-connector for its peer identity and outputs
# the DNS TXT record to add at _wws._tcp.<domain>.
#
# Arguments:
#   RPC_ADDR  — JSON-RPC address of the connector (default: 127.0.0.1:9370)
#   DOMAIN    — DNS domain to configure (default: worldwideswarm.net)

set -euo pipefail

RPC_ADDR="${1:-127.0.0.1:9370}"
DOMAIN="${2:-worldwideswarm.net}"

echo "=== WWS DNS Bootstrap Setup ==="
echo "RPC endpoint: $RPC_ADDR"
echo "DNS domain:   $DOMAIN"
echo

# Query connector identity
RESPONSE=$(curl -s -X POST "http://$RPC_ADDR" \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","method":"swarm.get_identity","params":{},"id":"1","signature":""}')

AGENT_DID=$(echo "$RESPONSE" | grep -o '"agent_id":"[^"]*"' | head -1 | cut -d'"' -f4)
PEER_ID=$(echo "$AGENT_DID" | sed 's/did:swarm://')

if [ -z "$PEER_ID" ]; then
  echo "ERROR: Could not extract peer ID from connector at $RPC_ADDR"
  echo "Response: $RESPONSE"
  exit 1
fi

echo "Peer ID: $PEER_ID"
echo

# Get the listen address
STATUS=$(curl -s -X POST "http://$RPC_ADDR" \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","method":"swarm.get_status","params":{},"id":"1","signature":""}')

echo "=== DNS TXT Record to Add ==="
echo
echo "Record name:  _wws._tcp.$DOMAIN"
echo "Record type:  TXT"
echo "Record value: \"v=1 peer=/dns4/$DOMAIN/tcp/9000/p2p/$PEER_ID\""
echo
echo "If your node uses a specific IP instead of a DNS name:"
echo "Record value: \"v=1 peer=/ip4/<YOUR_PUBLIC_IP>/tcp/<P2P_PORT>/p2p/$PEER_ID\""
echo
echo "=== Verification ==="
echo "After adding the record, verify with:"
echo "  dig TXT _wws._tcp.$DOMAIN"
echo "  # or"
echo "  nslookup -type=TXT _wws._tcp.$DOMAIN"
echo
echo "=== Quick Test ==="
echo "Start a fresh connector with zero config to test discovery:"
echo "  ./wws-connector --agent-name test-node"
echo "  # Should log: 'Found bootstrap peers via DNS TXT'"
