#!/bin/bash

# WorldWideSwarm Multi-Node Manager
# Start, stop, and manage multiple connector instances

set -e

# Add cargo to PATH
export PATH="$HOME/.cargo/bin:$PATH"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

SWARM_DIR="/tmp/wws-swarm"
NODES_FILE="$SWARM_DIR/nodes.txt"

# Load shared environment and config defaults
if [ -f "./scripts/load-env.sh" ]; then
    # shellcheck disable=SC1091
    source "./scripts/load-env.sh"
fi

# Default configuration
AGENT_IMPL=${AGENT_IMPL:-zeroclaw}
LLM_BACKEND=${LLM_BACKEND:-openrouter}
LOCAL_MODEL_PATH=${LOCAL_MODEL_PATH:-./models/gpt-oss-20b.gguf}
MODEL_NAME=${MODEL_NAME:-arcee-ai/trinity-large-preview:free}
ZEROCLAW_AUTO_UPDATE=${ZEROCLAW_AUTO_UPDATE:-true}

# Nobel laureate-inspired names used for swarm agents.
declare -a NOBEL_LAUREATE_NAMES=(
    "marie-curie" "albert-einstein" "niels-bohr" "max-planck" "werner-heisenberg"
    "paul-dirac" "erwin-schrodinger" "enrico-fermi" "richard-feynman" "murray-gell-mann"
    "abdus-salam" "steven-weinberg" "sheldon-glashow" "peter-higgs" "francois-englert"
    "donna-strickland" "andre-geim" "konstantin-novoselov" "robert-laughlin" "alexei-abrikosov"
    "vitaly-ginzburg" "serge-haroche" "david-wineland" "klaus-hasselmann" "giorgio-parisi"
    "syukuro-manabe" "john-bardeen" "walter-brattain" "william-shockley" "charles-townes"
    "ahmed-zewail" "roald-hoffmann" "linus-pauling" "dorothy-hodgkin" "frances-arnold"
    "jennifer-doudna" "emmanuelle-charpentier" "roger-penrose" "reinhard-genzel" "andrea-ghez"
    "paul-romer" "william-nordhaus" "elinor-ostrom" "amartya-sen" "joseph-stiglitz"
    "paul-krugman" "milton-friedman" "kenneth-arrow" "eugene-fama" "robert-shiller"
    "daniel-kahneman" "roger-myre-son" "esther-duflo" "abhijit-banerjee" "michael-kremer"
    "katalin-kariko" "drew-weissman" "tu-youyou" "svante-paabo" "randy-schekman"
)

nobel_agent_name_for_index() {
    local idx=$1
    local total=${#NOBEL_LAUREATE_NAMES[@]}
    if [ "$idx" -le "$total" ]; then
        echo "${NOBEL_LAUREATE_NAMES[$((idx-1))]}"
    else
        echo "${NOBEL_LAUREATE_NAMES[$(( (idx-1) % total ))]}-$idx"
    fi
}

usage() {
    cat << EOF
${GREEN}WorldWideSwarm Multi-Node Manager${NC}

Usage: $0 <command> [options]

Commands:
    start <N>           Start N connector nodes (default: 3)
    start-agents <N>    Start N full agents (connector + <Agent>) (default: 3)
    stop                Stop all running nodes and agents
    status              Show status of all nodes and agents
    test                Run a quick API test on all nodes
    clean               Clean up all temporary files
    help                Show this help message

Environment Variables:
    AGENT_IMPL          Agent implementation: claude-code-cli (default) | opencode | zeroclaw
    LLM_BACKEND         LLM backend for Zeroclaw: anthropic | openai | openrouter | local | ollama
    LOCAL_MODEL_PATH    Path to local model file (for local backend)
    MODEL_NAME          Model name (for OpenAI/OpenRouter/Ollama backends)

Examples:
    # Start 3 connector nodes
    $0 start 3

    # Start 3 full agents with Claude Code CLI (default)
    $0 start-agents 3

    # Start 5 full agents with OpenCode (uses opencode CLI + Anthropic subscription)
    AGENT_IMPL=opencode $0 start-agents 5

    # Start 15 agents with Zeroclaw + local LLM
    AGENT_IMPL=zeroclaw LLM_BACKEND=local $0 start-agents 15

    # Start agents with Zeroclaw + Ollama
    AGENT_IMPL=zeroclaw LLM_BACKEND=ollama $0 start-agents 15

    # Check status
    $0 status

    # Test all nodes
    $0 test

    # Stop all nodes and agents
    $0 stop

EOF
    exit 0
}

# Initialize swarm directory
init_swarm_dir() {
    mkdir -p "$SWARM_DIR"
    if [ ! -f "$NODES_FILE" ]; then
        touch "$NODES_FILE"
    fi
}

# Start N nodes
start_nodes() {
    local num_nodes=${1:-3}

    echo -e "${GREEN}Starting $num_nodes WorldWideSwarm nodes...${NC}"
    echo ""

    # Build the connector if not already built
    if [ ! -f "target/release/wws-connector" ]; then
        echo -e "${YELLOW}Building WorldWideSwarm connector...${NC}"
        cargo build --release
    fi

    init_swarm_dir

    # Clear existing nodes file
    > "$NODES_FILE"

    local bootstrap_addr="${BOOTSTRAP_ADDR:-}"

    for i in $(seq 1 $num_nodes); do
        local node_name="swarm-node-$i"
        local log_file="$SWARM_DIR/$node_name.log"

        echo -e "${BLUE}Starting node $i/$num_nodes: $node_name${NC}"

        # Find available ports
        local p2p_port=$(find_available_port $((9000 + i - 1)))
        local rpc_port=$(find_available_port $((9370 + i - 1)))

        # Build command
        local cmd="./target/release/wws-connector"
        cmd="$cmd --listen /ip4/0.0.0.0/tcp/$p2p_port"
        cmd="$cmd --rpc 127.0.0.1:$rpc_port"
        cmd="$cmd --agent-name $node_name"

        # Add bootstrap peer if not the first node
        if [ -n "$bootstrap_addr" ]; then
            cmd="$cmd --bootstrap $bootstrap_addr"
        fi

        # Start the node in background
        eval "$cmd > $log_file 2>&1 &"
        local pid=$!

        # Save node info
        echo "$node_name|$pid|$p2p_port|$rpc_port" >> "$NODES_FILE"

        # Wait for node to start
        sleep 2

        # Get peer ID for bootstrap
        if [ $i -eq 1 ]; then
            local peer_id=$(get_peer_id $rpc_port)
            if [ -n "$peer_id" ]; then
                bootstrap_addr="/ip4/127.0.0.1/tcp/$p2p_port/p2p/$peer_id"
                echo -e "  ${GREEN}✓${NC} Bootstrap node ready: $bootstrap_addr"
            fi
        else
            echo -e "  ${GREEN}✓${NC} Node started (PID: $pid, RPC: $rpc_port)"
        fi
    done

    echo ""
    echo -e "${GREEN}All $num_nodes nodes started successfully!${NC}"
    echo ""
    echo -e "${YELLOW}Node information saved to: $NODES_FILE${NC}"
    echo -e "${YELLOW}View status with: $0 status${NC}"
    echo ""
}

# Find available port
find_available_port() {
    local start_port=$1
    local port=$start_port
    while true; do
        if ! lsof -Pi :$port -sTCP:LISTEN -t >/dev/null 2>&1; then
            echo $port
            return
        fi
        port=$((port + 1))
    done
}

# Get peer ID from RPC
get_peer_id() {
    local rpc_port=$1
    echo '{"jsonrpc":"2.0","method":"swarm.get_status","params":{},"id":"1","signature":""}' | \
        nc 127.0.0.1 $rpc_port 2>/dev/null | \
        grep -o 'did:swarm:[^"]*' | \
        sed 's/did:swarm://' | \
        head -1
}

# Stop all nodes
stop_nodes() {
    init_swarm_dir

    echo -e "${YELLOW}Stopping all WorldWideSwarm nodes and agents...${NC}"
    echo ""

    # Read with updated format (supports both old and new format)
    while IFS='|' read -r name connector_pid p2p_port_or_claude rpc_port_or_p2p files_or_rpc files_port; do
        # Detect format (old: 4 fields, new: 6 fields)
        if [ -z "$files_or_rpc" ]; then
            # Old format: name|pid|p2p_port|rpc_port
            local pid=$connector_pid
            if ps -p $pid > /dev/null 2>&1; then
                echo -e "  Stopping $name (PID: $pid)..."
                kill $pid 2>/dev/null || true
            fi
        else
            # New format: name|connector_pid|claude_pid|p2p_port|rpc_port|files_port
            local claude_pid=$p2p_port_or_claude
            # Stop <Agent> agent if running
            if [ "$claude_pid" != "0" ] && ps -p $claude_pid > /dev/null 2>&1; then
                echo -e "  Stopping Claude agent for $name (PID: $claude_pid)..."
                kill $claude_pid 2>/dev/null || true
            fi
            # Stop connector
            if ps -p $connector_pid > /dev/null 2>&1; then
                echo -e "  Stopping connector $name (PID: $connector_pid)..."
                kill $connector_pid 2>/dev/null || true
            fi
        fi
    done < "$NODES_FILE"

    # Stop dedicated web-console connectors if present.
    if [ -f "$SWARM_DIR/operator-web-30.pid" ]; then
        local web30_pid
        web30_pid=$(cat "$SWARM_DIR/operator-web-30.pid" 2>/dev/null || true)
        if [ -n "$web30_pid" ] && ps -p $web30_pid > /dev/null 2>&1; then
            echo -e "  Stopping operator-web-30 (PID: $web30_pid)..."
            kill $web30_pid 2>/dev/null || true
        fi
        rm -f "$SWARM_DIR/operator-web-30.pid"
    fi

    if [ -f "$SWARM_DIR/operator-web-15.pid" ]; then
        local web15_pid
        web15_pid=$(cat "$SWARM_DIR/operator-web-15.pid" 2>/dev/null || true)
        if [ -n "$web15_pid" ] && ps -p $web15_pid > /dev/null 2>&1; then
            echo -e "  Stopping operator-web-15 (PID: $web15_pid)..."
            kill $web15_pid 2>/dev/null || true
        fi
        rm -f "$SWARM_DIR/operator-web-15.pid"
    fi

    # Safety net: terminate orphaned WorldWideSwarm processes not present in nodes.txt.
    local orphan_connectors
    orphan_connectors=$(pgrep -f 'wws-connector' || true)
    if [ -n "$orphan_connectors" ]; then
        echo -e "  Cleaning orphan connectors: $orphan_connectors"
        kill $orphan_connectors 2>/dev/null || true
    fi

    local orphan_zeroclaw
    orphan_zeroclaw=$(pgrep -f 'zeroclaw-agent.sh' || true)
    if [ -n "$orphan_zeroclaw" ]; then
        echo -e "  Cleaning orphan zeroclaw launchers: $orphan_zeroclaw"
        kill $orphan_zeroclaw 2>/dev/null || true
    fi

    # Wait for processes to terminate
    sleep 1

    echo ""
    echo -e "${GREEN}All nodes and agents stopped.${NC}"

    # Clear nodes file
    > "$NODES_FILE"
}

# Show status of all nodes
show_status() {
    init_swarm_dir

    if [ ! -s "$NODES_FILE" ]; then
        echo -e "${YELLOW}No nodes are currently running.${NC}"
        return
    fi

    echo -e "${GREEN}╔════════════════════════════════════════════════════════════════════════════════════════╗${NC}"
    echo -e "${GREEN}║                           WorldWideSwarm Nodes Status                                       ║${NC}"
    echo -e "${GREEN}╚════════════════════════════════════════════════════════════════════════════════════════╝${NC}"
    echo ""

    printf "%-20s %-10s %-10s %-10s %-10s %-12s %-10s\n" "NODE NAME" "CONN PID" "CLAUDE PID" "P2P PORT" "RPC PORT" "STATUS" "PEERS"
    printf "%-20s %-10s %-10s %-10s %-10s %-12s %-10s\n" "────────────────────" "──────────" "──────────" "──────────" "──────────" "────────────" "──────────"

    while IFS='|' read -r name connector_pid claude_pid_or_p2p p2p_port_or_rpc rpc_port_or_files files_port; do
        # Detect format (old: 4 fields, new: 6 fields)
        if [ -z "$rpc_port_or_files" ]; then
            # Old format: name|pid|p2p_port|rpc_port
            local pid=$connector_pid
            local p2p_port=$claude_pid_or_p2p
            local rpc_port=$p2p_port_or_rpc
            local claude_pid="N/A"
            local status="STOPPED"
            local peers="N/A"

            if ps -p $pid > /dev/null 2>&1; then
                status="${GREEN}RUNNING${NC}"

                # Get network stats
                local stats=$(echo '{"jsonrpc":"2.0","method":"swarm.get_network_stats","params":{},"id":"1","signature":""}' | nc 127.0.0.1 $rpc_port 2>/dev/null || echo "")

                if [ -n "$stats" ]; then
                    peers=$(echo "$stats" | grep -o '"total_agents":[0-9]*' | cut -d':' -f2)
                fi
            fi

            printf "%-20s %-10s %-10s %-10s %-10s %-12b %-10s\n" "$name" "$pid" "$claude_pid" "$p2p_port" "$rpc_port" "$status" "$peers"
        else
            # New format: name|connector_pid|claude_pid|p2p_port|rpc_port|files_port
            local claude_pid=$claude_pid_or_p2p
            local p2p_port=$p2p_port_or_rpc
            local rpc_port=$rpc_port_or_files
            local conn_status="STOPPED"
            local claude_status=""
            local peers="N/A"

            # Check connector status
            if ps -p $connector_pid > /dev/null 2>&1; then
                conn_status="${GREEN}CONN:OK${NC}"

                # Get network stats
                local stats=$(echo '{"jsonrpc":"2.0","method":"swarm.get_network_stats","params":{},"id":"1","signature":""}' | nc 127.0.0.1 $rpc_port 2>/dev/null || echo "")

                if [ -n "$stats" ]; then
                    peers=$(echo "$stats" | grep -o '"total_agents":[0-9]*' | cut -d':' -f2)
                fi
            fi

            # Check <Agent> status
            if [ "$claude_pid" != "0" ] && ps -p $claude_pid > /dev/null 2>&1; then
                claude_status=" ${GREEN}CLI:OK${NC}"
            elif [ "$claude_pid" != "0" ]; then
                claude_status=" ${RED}CLI:OFF${NC}"
            fi

            local combined_status="${conn_status}${claude_status}"

            printf "%-20s %-10s %-10s %-10s %-10s %-12b %-10s\n" "$name" "$connector_pid" "$claude_pid" "$p2p_port" "$rpc_port" "$combined_status" "$peers"
        fi
    done < "$NODES_FILE"

    echo ""
}

# Test all nodes
test_nodes() {
    init_swarm_dir

    if [ ! -s "$NODES_FILE" ]; then
        echo -e "${YELLOW}No nodes are currently running.${NC}"
        return
    fi

    echo -e "${GREEN}Testing all WorldWideSwarm nodes...${NC}"
    echo ""

    while IFS='|' read -r name connector_pid claude_pid_or_p2p p2p_port_or_rpc rpc_port_or_files files_port; do
        # Detect format (old: 4 fields, new: 6 fields)
        if [ -z "$rpc_port_or_files" ]; then
            # Old format: name|pid|p2p_port|rpc_port
            local pid=$connector_pid
            local rpc_port=$p2p_port_or_rpc
        else
            # New format: name|connector_pid|claude_pid|p2p_port|rpc_port|files_port
            local pid=$connector_pid
            local claude_pid=$claude_pid_or_p2p
            local rpc_port=$rpc_port_or_files
        fi

        echo -e "${BLUE}Testing $name (RPC: $rpc_port)${NC}"

        if ! ps -p $pid > /dev/null 2>&1; then
            echo -e "  ${RED}✗ Connector is not running${NC}"
            echo ""
            continue
        fi

        # Test get_status
        local response=$(echo '{"jsonrpc":"2.0","method":"swarm.get_status","params":{},"id":"1","signature":""}' | nc 127.0.0.1 $rpc_port 2>/dev/null)

        if echo "$response" | grep -q "result"; then
            local agent_id=$(echo "$response" | grep -o 'did:swarm:[^"]*' | head -1)
            local tier=$(echo "$response" | grep -o '"tier":"[^"]*"' | cut -d':' -f2 | tr -d '"')
            local epoch=$(echo "$response" | grep -o '"epoch":[0-9]*' | cut -d':' -f2)

            echo -e "  ${GREEN}✓ Connector API responding${NC}"
            echo -e "    Agent ID: $agent_id"
            echo -e "    Tier: $tier"
            echo -e "    Epoch: $epoch"

            # Check <Agent> status if present
            if [ -n "$claude_pid" ] && [ "$claude_pid" != "0" ]; then
                if ps -p $claude_pid > /dev/null 2>&1; then
                    echo -e "  ${GREEN}✓ <Agent> agent running${NC} (PID: $claude_pid)"
                else
                    echo -e "  ${RED}✗ <Agent> agent not running${NC}"
                fi
            fi
        else
            echo -e "  ${RED}✗ API not responding${NC}"
        fi

        echo ""
    done
}

# Clean up
clean_up() {
    echo -e "${YELLOW}Cleaning up WorldWideSwarm temporary files...${NC}"

    # Stop nodes first
    stop_nodes

    # Remove swarm directory
    if [ -d "$SWARM_DIR" ]; then
        rm -rf "$SWARM_DIR"
        echo -e "${GREEN}Temporary files cleaned.${NC}"
    fi

    # Remove other temp files
    rm -f /tmp/wws-*.pid
    rm -f /tmp/wws-*-info.txt
    rm -f /tmp/node*.log

    echo -e "${GREEN}Cleanup complete.${NC}"
}

# Start N full agents (connector + <Agent>)
start_agents() {
    local num_agents=${1:-3}

    echo -e "${GREEN}Starting $num_agents WorldWideSwarm agents (connector + <Agent>)...${NC}"
    echo ""

    # Build the connector if not already built
    if [ ! -f "target/release/wws-connector" ]; then
        echo -e "${YELLOW}Building WorldWideSwarm connector...${NC}"
        cargo build --release
    fi

    init_swarm_dir

    # Clear existing nodes file
    > "$NODES_FILE"

    local bootstrap_addr=""

    if [ "$AGENT_IMPL" = "zeroclaw" ] && [ "$ZEROCLAW_AUTO_UPDATE" = "true" ]; then
        if [ -x "./scripts/update-zeroclaw.sh" ]; then
            echo -e "${BLUE}Updating Zeroclaw to latest version...${NC}"
            ./scripts/update-zeroclaw.sh || {
                echo -e "${YELLOW}Warning: Zeroclaw update failed, continuing with installed version.${NC}"
            }
            echo ""
        fi
    fi

    if [ "$AGENT_IMPL" = "zeroclaw" ] && [ "$LLM_BACKEND" = "ollama" ]; then
        if [ -x "./scripts/setup-local-llm.sh" ]; then
            echo -e "${BLUE}Ensuring local Ollama model server is running...${NC}"
            ./scripts/setup-local-llm.sh start >/dev/null || {
                echo -e "${RED}Failed to start local model server via scripts/setup-local-llm.sh${NC}"
                exit 1
            }
            echo -e "${GREEN}✓${NC} Local model server ready"
            echo ""
        fi
    fi

    if [ "$AGENT_IMPL" = "zeroclaw" ] && [ "$LLM_BACKEND" = "local" ]; then
        if [ -x "./scripts/setup-local-llm.sh" ]; then
            echo -e "${BLUE}Ensuring local llama.cpp model server is running...${NC}"
            ./scripts/setup-local-llm.sh start --backend llamacpp >/dev/null || {
                echo -e "${RED}Failed to start local llama.cpp server via scripts/setup-local-llm.sh${NC}"
                exit 1
            }
            echo -e "${GREEN}✓${NC} Local llama.cpp server ready"
            echo ""
        fi
    fi

    for i in $(seq 1 $num_agents); do
        local agent_name
        agent_name="$(nobel_agent_name_for_index "$i")"
        local log_file="$SWARM_DIR/$agent_name.log"

        echo -e "${BLUE}Starting agent $i/$num_agents: $agent_name${NC}"

        # Find available ports
        local p2p_port=$(find_available_port $((9000 + i - 1)))
        local rpc_port=$(find_available_port $((9370 + i - 1)))
        # Ensure FILES_PORT is different from RPC_PORT
        local files_port=$(find_available_port $((rpc_port + 1)))

        # Build connector command
        local connector_cmd="./target/release/wws-connector"
        connector_cmd="$connector_cmd --listen /ip4/127.0.0.1/tcp/$p2p_port"
        connector_cmd="$connector_cmd --rpc 127.0.0.1:$rpc_port"
        connector_cmd="$connector_cmd --files-addr 127.0.0.1:$files_port"
        connector_cmd="$connector_cmd --agent-name $agent_name"

        # Add bootstrap peer if not the first agent
        if [ -n "$bootstrap_addr" ]; then
            connector_cmd="$connector_cmd --bootstrap $bootstrap_addr"
        fi

        # Start the connector in background
        eval "$connector_cmd > $log_file 2>&1 &"
        local connector_pid=$!

        # Save node info (format: name|connector_pid|claude_pid|p2p_port|rpc_port|files_port)
        echo "$agent_name|$connector_pid|0|$p2p_port|$rpc_port|$files_port" >> "$NODES_FILE"

        # Wait for connector to start
        sleep 3

        # Check if connector is still running
        if ! ps -p $connector_pid > /dev/null 2>&1; then
            echo -e "  ${RED}✗ Connector failed to start. Check logs at $log_file${NC}"
            continue
        fi

        echo -e "  ${GREEN}✓${NC} Connector started (PID: $connector_pid, RPC: $rpc_port, Files: $files_port)"

        # Force explicit RPC connect to bootstrap node to improve convergence.
        if [ -n "$bootstrap_addr" ]; then
            for connect_try in {1..5}; do
                local connect_resp=$(echo "{\"jsonrpc\":\"2.0\",\"method\":\"swarm.connect\",\"params\":{\"addr\":\"$bootstrap_addr\"},\"id\":\"connect\",\"signature\":\"\"}" | nc 127.0.0.1 "$rpc_port" 2>/dev/null || true)
                if echo "$connect_resp" | grep -q '"connected":true'; then
                    echo -e "  ${GREEN}✓${NC} Connected to bootstrap node"
                    break
                fi
                sleep 1
            done
        fi

        # Wait for file server to be ready
        echo -e "  ${BLUE}Waiting for file server...${NC}"
        for j in {1..10}; do
            if curl -s "http://127.0.0.1:$files_port/SKILL.md" > /dev/null 2>&1; then
                echo -e "  ${GREEN}✓${NC} File server ready"
                break
            fi
            if [ $j -eq 10 ]; then
                echo -e "  ${YELLOW}⚠${NC} File server may not be ready yet"
            fi
            sleep 1
        done

        # For the first agent, get peer ID for bootstrap when external bootstrap is not set
        if [ $i -eq 1 ] && [ -z "$bootstrap_addr" ]; then
            local peer_id=$(get_peer_id $rpc_port)
            if [ -n "$peer_id" ]; then
                bootstrap_addr="/ip4/127.0.0.1/tcp/$p2p_port/p2p/$peer_id"
                echo -e "  ${GREEN}✓${NC} Bootstrap agent ready: $bootstrap_addr"
            fi
        fi

        # Build agent instruction for LLM-based implementations
        local claude_instruction="CRITICAL: You are a LONG-RUNNING AUTONOMOUS AGENT. Run in an INFINITE LOOP until interrupted.

Fetch the WorldWideSwarm skill documentation using curl (use Bash tool, not Fetch tool):

curl http://127.0.0.1:$files_port/SKILL.md

Read the SKILL.md file carefully. Pay special attention to the 'CRITICAL: CONTINUOUS OPERATION' section.

You are agent '$agent_name'. Your job is to run CONTINUOUSLY in an infinite loop:

STEP 1 - INITIALIZE (run once):
- Register yourself: echo '{\"jsonrpc\":\"2.0\",\"method\":\"swarm.register_agent\",\"params\":{\"agent_id\":\"$agent_name\"},\"id\":\"1\",\"signature\":\"\"}' | nc 127.0.0.1 $rpc_port
- Get your status: echo '{\"jsonrpc\":\"2.0\",\"method\":\"swarm.get_status\",\"params\":{},\"id\":\"status\",\"signature\":\"\"}' | nc 127.0.0.1 $rpc_port
- Parse the response to learn your tier: \"tier\": \"Tier1\" / \"Tier2\" / \"Tier3\" / ... / \"Executor\"
- Store your tier in memory - you'll need it for task processing
- IMPORTANT: Any tier except \"Executor\" is a COORDINATOR tier

STEP 2 - INFINITE LOOP (run forever):
Create an infinite while loop that polls every 60 seconds:

A. Poll for tasks: echo '{\"jsonrpc\":\"2.0\",\"method\":\"swarm.receive_task\",\"params\":{},\"id\":\"2\",\"signature\":\"\"}' | nc 127.0.0.1 $rpc_port

B. Track which task IDs you've already processed (keep a list in memory)

C. For each NEW task (not already processed):
   - Get task details using swarm.get_task

   - **IF YOUR TIER IS A COORDINATOR (Tier1, Tier2, Tier3, ..., TierN - anything except Executor):**
     * You are a coordinator! Your job is to DECOMPOSE tasks into subtasks.
     * Generate a decomposition plan using AI reasoning:
       1. Analyze the task description, complexity, and scope
       2. Consider your position in hierarchy:
          - Tier-1 (top): Break complex projects into major components
          - Tier-2/3/N (middle): Break components into smaller work units
          - Deeper tiers: Break into increasingly atomic tasks
       3. Decide number of subtasks (typically 3-10 for good parallelism)
       4. For each subtask, write a CLEAR, SPECIFIC description
       5. Create a plan JSON with this structure:
          {
            \"task_id\": \"<task_id>\",
            \"proposer\": \"<your agent_id from status>\",
            \"epoch\": <epoch from task>,
            \"subtasks\": [
              {\"index\": 1, \"description\": \"Detailed subtask 1\", \"estimated_complexity\": 0.2},
              {\"index\": 2, \"description\": \"Detailed subtask 2\", \"estimated_complexity\": 0.3},
              ...
            ],
            \"rationale\": \"Why this decomposition is optimal\",
            \"estimated_parallelism\": <number of subtasks>
          }
       6. Submit your plan: echo '{\"jsonrpc\":\"2.0\",\"method\":\"swarm.propose_plan\",\"params\":<plan_json>,\"id\":\"propose\",\"signature\":\"\"}' | nc 127.0.0.1 $rpc_port
       7. Your plan competes with other coordinators at your tier via voting
     * After submitting plan, add task ID to your processed list
     * THE RECURSIVE PATTERN: Your subtasks may become tasks for lower-tier coordinators!

   - **IF YOUR TIER IS 'Executor' (Leaf Worker - bottom of hierarchy):**
     * You are a worker! Your job is to EXECUTE tasks, not decompose them.
     * Perform the actual work described in the task:
       1. Read and analyze the task description carefully
       2. Use your AI capabilities to complete the task (research, write code, analyze data, etc.)
       3. Generate a comprehensive result artifact
       4. Compute a content ID (simple hash of your result)
     * Create result JSON:
       {
         \"task_id\": \"<task_id>\",
         \"agent_id\": \"<your agent_id>\",
         \"artifact\": {
           \"artifact_id\": \"<task_id>-result\",
           \"task_id\": \"<task_id>\",
           \"producer\": \"<your agent_id>\",
           \"content_cid\": \"<hash of your result>\",
           \"merkle_hash\": \"<hash of your result>\",
           \"content_type\": \"text/plain\",
           \"size_bytes\": <size of result>,
           \"created_at\": \"<current timestamp>\"
         },
         \"merkle_proof\": []
       }
     * Submit result: echo '{\"jsonrpc\":\"2.0\",\"method\":\"swarm.submit_result\",\"params\":<result_json>,\"id\":\"result\",\"signature\":\"\"}' | nc 127.0.0.1 $rpc_port
     * Add task ID to your processed list
     * IMPORTANT: Provide meaningful, high-quality work - this is the actual execution!

D. Sleep 60 seconds and repeat

IMPORTANT:
- NEVER process the same task twice - track completed task IDs
- Use Bash tool for all RPC calls (nc/curl, NOT Fetch tool)
- Show all actions and responses for debugging
- Only stop when interrupted with Ctrl+C
- Tier1 agents propose PLANS, Executors perform WORK

RPC server: tcp://127.0.0.1:$rpc_port

Work autonomously. Run the infinite loop until interrupted."

        # Start AI agent (Claude Code CLI, OpenCode, Zeroclaw, or none)
        if [ "$AGENT_IMPL" = "none" ]; then
            echo -e "  ${YELLOW}ℹ${NC} AGENT_IMPL=none — connector only, no agent started"
            # agent_pid stays 0 in nodes.txt; nothing to update
        elif [ "$AGENT_IMPL" = "zeroclaw" ]; then
            echo -e "  ${BLUE}Starting Zeroclaw agent (LLM: $LLM_BACKEND, model: $MODEL_NAME)...${NC}"

            # Check if zeroclaw launcher exists
            if [ ! -f "agent-impl/zeroclaw/zeroclaw-agent.sh" ]; then
                echo -e "  ${RED}✗${NC} Zeroclaw launcher not found at agent-impl/zeroclaw/zeroclaw-agent.sh"
                continue
            fi

            local zeroclaw_log_file="$SWARM_DIR/$agent_name-zeroclaw.log"
            ./agent-impl/zeroclaw/zeroclaw-agent.sh \
                --agent-name "$agent_name" \
                --rpc-port "$rpc_port" \
                --files-port "$files_port" \
                --llm-backend "$LLM_BACKEND" \
                --model-path "$LOCAL_MODEL_PATH" \
                --model-name "$MODEL_NAME" \
                > "$zeroclaw_log_file" 2>&1 &
            local agent_pid=$!

            # Wait to check if started
            sleep 1
            if ps -p $agent_pid > /dev/null 2>&1; then
                if [[ "$OSTYPE" == "darwin"* ]]; then
                    sed -i '' "s/$agent_name|$connector_pid|0|/$agent_name|$connector_pid|$agent_pid|/" "$NODES_FILE"
                else
                    sed -i "s/$agent_name|$connector_pid|0|/$agent_name|$connector_pid|$agent_pid|/" "$NODES_FILE"
                fi
                echo -e "  ${GREEN}✓${NC} Zeroclaw agent started (PID: $agent_pid)"
            else
                echo -e "  ${RED}✗${NC} Zeroclaw agent failed to start (check log: $zeroclaw_log_file)"
            fi
        elif [ "$AGENT_IMPL" = "opencode" ]; then
            echo -e "  ${BLUE}Starting OpenCode agent (model: ${OPENCODE_MODEL:-openai/gpt-5.2-codex})...${NC}"

            if [ ! -f "agent-impl/opencode/opencode-agent.sh" ]; then
                echo -e "  ${RED}✗${NC} OpenCode launcher not found at agent-impl/opencode/opencode-agent.sh"
                continue
            fi

            local opencode_log_file="$SWARM_DIR/$agent_name-opencode.log"
            OPENCODE_BIN="${OPENCODE_BIN:-/opt/homebrew/bin/opencode}" \
            OPENCODE_MODEL="${OPENCODE_MODEL:-openai/gpt-5.2-codex}" \
            POLL_INTERVAL="${POLL_INTERVAL:-30}" \
            ./agent-impl/opencode/opencode-agent.sh \
                "$agent_name" "$rpc_port" "$files_port" \
                > "$opencode_log_file" 2>&1 &
            local agent_pid=$!

            sleep 2
            if ps -p $agent_pid > /dev/null 2>&1; then
                if [[ "$OSTYPE" == "darwin"* ]]; then
                    sed -i '' "s/$agent_name|$connector_pid|0|/$agent_name|$connector_pid|$agent_pid|/" "$NODES_FILE"
                else
                    sed -i "s/$agent_name|$connector_pid|0|/$agent_name|$connector_pid|$agent_pid|/" "$NODES_FILE"
                fi
                echo -e "  ${GREEN}✓${NC} OpenCode agent started (PID: $agent_pid)"
            else
                echo -e "  ${RED}✗${NC} OpenCode agent failed to start (check: $opencode_log_file)"
            fi
        else
            echo -e "  ${BLUE}Starting Claude Code CLI agent...${NC}"

            # Launch Claude Code CLI in background
            local claude_log_file="$SWARM_DIR/$agent_name-claude.log"
            # Preserve ANTHROPIC_* environment variables for authentication
            env -u CLAUDECODE \
                ANTHROPIC_AUTH_TOKEN="${ANTHROPIC_AUTH_TOKEN:-}" \
                ANTHROPIC_BASE_URL="${ANTHROPIC_BASE_URL:-}" \
                ANTHROPIC_MODEL="${ANTHROPIC_MODEL:-}" \
                ANTHROPIC_DEFAULT_HAIKU_MODEL="${ANTHROPIC_DEFAULT_HAIKU_MODEL:-}" \
                claude --dangerously-skip-permissions "$claude_instruction" > "$claude_log_file" 2>&1 &
            local claude_pid=$!

            # Wait a moment to check if Claude starts successfully
            sleep 1
            if ps -p $claude_pid > /dev/null 2>&1; then
                # Update the nodes file with Claude PID
                if [[ "$OSTYPE" == "darwin"* ]]; then
                    sed -i '' "s/$agent_name|$connector_pid|0|/$agent_name|$connector_pid|$claude_pid|/" "$NODES_FILE"
                else
                    sed -i "s/$agent_name|$connector_pid|0|/$agent_name|$connector_pid|$claude_pid|/" "$NODES_FILE"
                fi
                echo -e "  ${GREEN}✓${NC} <Agent> agent started (PID: $claude_pid)"
            else
                echo -e "  ${RED}✗${NC} <Agent> agent failed to start (check log: $claude_log_file)"
            fi
        fi  # End of agent implementation selection
        echo ""
    done

    echo ""
    echo -e "${GREEN}═══════════════════════════════════════════════════════════${NC}"
    echo -e "${GREEN}All $num_agents agents started successfully!${NC}"
    echo -e "${GREEN}═══════════════════════════════════════════════════════════${NC}"
    echo ""
    echo -e "${YELLOW}Agent information saved to: $NODES_FILE${NC}"
    echo -e "${YELLOW}View status with: $0 status${NC}"
    echo -e "${YELLOW}View Claude agent logs: tail -f $SWARM_DIR/swarm-agent-*-claude.log${NC}"
    echo ""
}

# Main command dispatcher
case "${1:-help}" in
    start)
        start_nodes ${2:-3}
        ;;
    start-agents)
        start_agents ${2:-3}
        ;;
    stop)
        stop_nodes
        ;;
    status)
        show_status
        ;;
    test)
        test_nodes
        ;;
    clean)
        clean_up
        ;;
    help|--help|-h)
        usage
        ;;
    *)
        echo -e "${RED}Unknown command: $1${NC}"
        echo ""
        usage
        ;;
esac
