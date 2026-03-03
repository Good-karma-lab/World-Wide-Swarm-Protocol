#!/bin/bash

# Show WorldWideSwarm hierarchy
# Usage: ./show-hierarchy.sh [rpc_port]

RPC_PORT=${1:-9370}

echo "╔═══════════════════════════════════════════════════════════════╗"
echo "║              WorldWideSwarm Agent Hierarchy                        ║"
echo "╚═══════════════════════════════════════════════════════════════╝"
echo ""

# Get hierarchy from RPC
RESPONSE=$(echo '{"jsonrpc":"2.0","method":"swarm.get_hierarchy","params":{},"id":"1","signature":""}' | nc 127.0.0.1 $RPC_PORT 2>/dev/null)

# Parse and display
echo "$RESPONSE" | jq -r '
.result |
"Total Agents: \(.total_agents)",
"Branching Factor: \(.branching_factor)",
"Hierarchy Depth: \(.hierarchy_depth)",
"Epoch: \(.epoch)",
"",
"Self:",
"  Agent: \(.self.agent_id)",
"  Tier: \(.self.tier)",
"  Tasks: \(.self.task_count)",
"",
"Peers:",
(.peers[] | "  • \(.agent_id) - Tier: \(.tier), Tasks: \(.task_count), Parent: \(.parent_id // "none")")
'

echo ""
echo "═══════════════════════════════════════════════════════════════"
