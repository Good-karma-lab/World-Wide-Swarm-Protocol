#!/bin/bash

# WorldWideSwarm Agent Runner
# Starts a swarm connector and an AI agent (Claude Code CLI or Zeroclaw) connected to it

set -e

# Add cargo to PATH if not already there
export PATH="$HOME/.cargo/bin:$PATH"

# Load shared environment and config defaults
if [ -f "./scripts/load-env.sh" ]; then
    # shellcheck disable=SC1091
    source "./scripts/load-env.sh"
fi

# Agent implementation: claude-code-cli (default) or zeroclaw
AGENT_IMPL=${AGENT_IMPL:-zeroclaw}
LLM_BACKEND=${LLM_BACKEND:-openrouter}
LOCAL_MODEL_PATH=${LOCAL_MODEL_PATH:-./models/gpt-oss-20b.gguf}
MODEL_NAME=${MODEL_NAME:-arcee-ai/trinity-large-preview:free}
ZEROCLAW_AUTO_UPDATE=${ZEROCLAW_AUTO_UPDATE:-true}

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Function to find an available port
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

# Function to get local IP
get_local_ip() {
    if [[ "$OSTYPE" == "darwin"* ]]; then
        ipconfig getifaddr en0 2>/dev/null || ipconfig getifaddr en1 2>/dev/null || echo "127.0.0.1"
    else
        hostname -I | awk '{print $1}' || echo "127.0.0.1"
    fi
}

# Parse command line arguments
AGENT_NAME=""
BOOTSTRAP_PEER=""
SWARM_ID="public"
CONNECTOR_ONLY=false
IDENTITY_PATH=""

usage() {
    cat << EOF
${GREEN}WorldWideSwarm Agent Runner${NC}

Starts a swarm connector node and an AI agent connected to it.

Usage: $0 [OPTIONS]

Options:
    -n, --name NAME          Agent name (default: auto-generated)
    -b, --bootstrap ADDR     Bootstrap peer multiaddress (optional; built-in defaults used if omitted)
    -s, --swarm-id ID        Swarm ID to join (default: public)
    --identity-path PATH     Path to identity key file (default: ~/.wws/<name>.key)
    --agent-impl IMPL        Agent implementation: claude-code-cli | zeroclaw (default: $AGENT_IMPL)
    --llm-backend BACKEND    LLM backend for Zeroclaw: anthropic | openai | openrouter | local | ollama (default: $LLM_BACKEND)
    --model-name NAME        Model name (default: $MODEL_NAME)
    --connector-only         Only run the connector (no AI agent)
    -h, --help               Show this help message

Examples:
    # Start with Claude Code CLI (default)
    $0 -n "alice"

    # Start with Zeroclaw + Ollama (recommended for local LLM)
    $0 -n "alice" --agent-impl zeroclaw --llm-backend ollama --model-name gpt-oss:20b

    # Start with Zeroclaw + local llama.cpp
    $0 -n "alice" --agent-impl zeroclaw --llm-backend local

    # Start with Zeroclaw + Claude API
    export ANTHROPIC_API_KEY="your-key"
    $0 -n "alice" --agent-impl zeroclaw --llm-backend anthropic

    # Environment variable configuration (recommended)
    export AGENT_IMPL=zeroclaw
    export LLM_BACKEND=ollama
    export MODEL_NAME=gpt-oss:20b
    $0 -n "alice"

    # Start and connect to existing swarm
    $0 -n "bob" -b "/ip4/127.0.0.1/tcp/9000/p2p/12D3Koo..."

    # Start only the connector
    $0 -n "connector-1" --connector-only

EOF
    exit 0
}

while [[ $# -gt 0 ]]; do
    case $1 in
        -n|--name)
            AGENT_NAME="$2"
            shift 2
            ;;
        -b|--bootstrap)
            BOOTSTRAP_PEER="$2"
            shift 2
            ;;
        -s|--swarm-id)
            SWARM_ID="$2"
            shift 2
            ;;
        --agent-impl)
            AGENT_IMPL="$2"
            shift 2
            ;;
        --llm-backend)
            LLM_BACKEND="$2"
            shift 2
            ;;
        --model-name)
            MODEL_NAME="$2"
            shift 2
            ;;
        --identity-path)
            IDENTITY_PATH="$2"
            shift 2
            ;;
        --connector-only)
            CONNECTOR_ONLY=true
            shift
            ;;
        -h|--help)
            usage
            ;;
        *)
            echo -e "${RED}Unknown option: $1${NC}"
            usage
            ;;
    esac
done

# Generate agent name if not provided
if [ -z "$AGENT_NAME" ]; then
    AGENT_NAME="agent-$(date +%s)-$$"
fi

# Default identity path to ~/.wws/<agent-name>.key
if [ -z "$IDENTITY_PATH" ]; then
    IDENTITY_PATH="$HOME/.wws/${AGENT_NAME}.key"
fi
mkdir -p "$HOME/.wws"

if [ "$AGENT_IMPL" = "zeroclaw" ] && [ "$ZEROCLAW_AUTO_UPDATE" = "true" ]; then
    if [ -x "./scripts/update-zeroclaw.sh" ]; then
        echo -e "${BLUE}Updating Zeroclaw to latest version...${NC}"
        ./scripts/update-zeroclaw.sh || {
            echo -e "${YELLOW}Warning: Zeroclaw update failed, continuing with installed version.${NC}"
        }
    fi
fi

if [ "$AGENT_IMPL" = "zeroclaw" ] && [ "$LLM_BACKEND" = "ollama" ]; then
    if [ -x "./scripts/setup-local-llm.sh" ]; then
        echo -e "${BLUE}Ensuring local Ollama model server is running...${NC}"
        ./scripts/setup-local-llm.sh start >/dev/null || {
            echo -e "${RED}Failed to start local model server via scripts/setup-local-llm.sh${NC}"
            exit 1
        }
    fi
fi

# Find available ports
echo -e "${BLUE}Finding available ports...${NC}"
P2P_PORT=$(find_available_port 9000)
RPC_PORT=$(find_available_port 9370)
# Ensure FILES_PORT is different from RPC_PORT
FILES_PORT=$(find_available_port $((RPC_PORT + 1)))

# Get local IP for display
LOCAL_IP=$(get_local_ip)

# Build the connector if not already built
if [ ! -f "target/release/wws-connector" ]; then
    echo -e "${YELLOW}Building WorldWideSwarm connector...${NC}"
    cargo build --release
fi

# Prepare connector command
CONNECTOR_CMD="./target/release/wws-connector"
CONNECTOR_CMD="$CONNECTOR_CMD --listen /ip4/0.0.0.0/tcp/$P2P_PORT"
CONNECTOR_CMD="$CONNECTOR_CMD --rpc 127.0.0.1:$RPC_PORT"
CONNECTOR_CMD="$CONNECTOR_CMD --files-addr 127.0.0.1:$FILES_PORT"
CONNECTOR_CMD="$CONNECTOR_CMD --agent-name \"$AGENT_NAME\""
CONNECTOR_CMD="$CONNECTOR_CMD --wws-name \"$AGENT_NAME\""
CONNECTOR_CMD="$CONNECTOR_CMD --identity-path \"$IDENTITY_PATH\""
CONNECTOR_CMD="$CONNECTOR_CMD --swarm-id \"$SWARM_ID\""

if [ -n "$BOOTSTRAP_PEER" ]; then
    CONNECTOR_CMD="$CONNECTOR_CMD --bootstrap \"$BOOTSTRAP_PEER\""
fi

# Display connection information
echo ""
echo -e "${GREEN}╔════════════════════════════════════════════════════════════╗${NC}"
echo -e "${GREEN}║         WorldWideSwarm Agent Starting...                       ║${NC}"
echo -e "${GREEN}╚════════════════════════════════════════════════════════════╝${NC}"
echo ""
echo -e "${BLUE}Agent Name:${NC}     $AGENT_NAME"
echo -e "${BLUE}Identity:${NC}       $IDENTITY_PATH"
if [ "$AGENT_IMPL" = "zeroclaw" ]; then
    echo -e "${BLUE}LLM Backend:${NC}    $LLM_BACKEND"
    echo -e "${BLUE}Model Name:${NC}     $MODEL_NAME"
fi
echo -e "${BLUE}Swarm ID:${NC}       $SWARM_ID"
echo -e "${BLUE}P2P Port:${NC}       $P2P_PORT"
echo -e "${BLUE}RPC Port:${NC}       $RPC_PORT"
echo -e "${BLUE}Files Port:${NC}     $FILES_PORT"
echo ""
echo -e "${YELLOW}Connection Information:${NC}"
echo -e "  ${BLUE}JSON-RPC API:${NC}  tcp://127.0.0.1:$RPC_PORT"
echo -e "  ${BLUE}File Server:${NC}   http://127.0.0.1:$FILES_PORT"
echo ""

# Save PID files for cleanup
CONNECTOR_PID_FILE="/tmp/wws-agent-$AGENT_NAME-connector.pid"
CLAUDE_PID_FILE="/tmp/wws-agent-$AGENT_NAME-claude.pid"

# Cleanup function
cleanup() {
    echo ""
    echo -e "${YELLOW}Shutting down agent...${NC}"

    if [ -f "$CLAUDE_PID_FILE" ]; then
        CLAUDE_PID=$(cat "$CLAUDE_PID_FILE")
        if ps -p $CLAUDE_PID > /dev/null 2>&1; then
            if [ "$AGENT_IMPL" = "zeroclaw" ]; then
                echo -e "${BLUE}Stopping Zeroclaw agent (PID: $CLAUDE_PID)...${NC}"
            else
                echo -e "${BLUE}Stopping <Agent> agent (PID: $CLAUDE_PID)...${NC}"
            fi
            kill $CLAUDE_PID 2>/dev/null || true
        fi
        rm -f "$CLAUDE_PID_FILE"
    fi

    if [ -f "$CONNECTOR_PID_FILE" ]; then
        CONNECTOR_PID=$(cat "$CONNECTOR_PID_FILE")
        if ps -p $CONNECTOR_PID > /dev/null 2>&1; then
            echo -e "${BLUE}Stopping connector (PID: $CONNECTOR_PID)...${NC}"
            kill $CONNECTOR_PID 2>/dev/null || true
        fi
        rm -f "$CONNECTOR_PID_FILE"
    fi

    echo -e "${GREEN}Agent stopped.${NC}"
    exit 0
}

trap cleanup INT TERM

# Start the connector in background
echo -e "${BLUE}Starting swarm connector...${NC}"
eval "$CONNECTOR_CMD" > "/tmp/wws-agent-$AGENT_NAME-connector.log" 2>&1 &
CONNECTOR_PID=$!
echo $CONNECTOR_PID > "$CONNECTOR_PID_FILE"

# Wait for connector to start
echo -e "${BLUE}Waiting for connector to initialize...${NC}"
sleep 3

# Check if connector is still running
if ! ps -p $CONNECTOR_PID > /dev/null 2>&1; then
    echo -e "${RED}✗ Connector failed to start. Check logs at /tmp/wws-agent-$AGENT_NAME-connector.log${NC}"
    exit 1
fi

echo -e "${GREEN}✓ Connector started successfully (PID: $CONNECTOR_PID)${NC}"

# Wait for file server to be ready
echo -e "${BLUE}Waiting for file server to be ready...${NC}"
for i in {1..10}; do
    if curl -s "http://127.0.0.1:$FILES_PORT/SKILL.md" > /dev/null 2>&1; then
        echo -e "${GREEN}✓ File server ready${NC}"
        break
    fi
    if [ $i -eq 10 ]; then
        echo -e "${YELLOW}⚠ File server may not be ready yet${NC}"
    fi
    sleep 1
done

# If connector-only mode, just wait
if [ "$CONNECTOR_ONLY" = true ]; then
    echo ""
    echo -e "${YELLOW}Running in connector-only mode. Press Ctrl+C to stop.${NC}"
    echo -e "${GREEN}════════════════════════════════════════════════════════════${NC}"
    echo ""
    wait $CONNECTOR_PID
    exit 0
fi

echo ""
echo -e "${GREEN}════════════════════════════════════════════════════════════${NC}"
if [ "$AGENT_IMPL" = "zeroclaw" ]; then
    echo -e "${GREEN}Starting WorldWideSwarm Agent (Connector + Zeroclaw)${NC}"
else
    echo -e "${GREEN}Starting WorldWideSwarm Agent (Connector + Claude Code CLI)${NC}"
fi
echo -e "${GREEN}════════════════════════════════════════════════════════════${NC}"
echo ""
echo -e "${BLUE}Connector:${NC}     Running on RPC port $RPC_PORT, Files port $FILES_PORT"
echo -e "${BLUE}Agent ID:${NC}      $AGENT_NAME"
echo -e "${BLUE}Agent Impl:${NC}    $AGENT_IMPL"
if [ "$AGENT_IMPL" = "zeroclaw" ]; then
    echo -e "${BLUE}LLM Backend:${NC}   $LLM_BACKEND"
    echo -e "${BLUE}Model:${NC}         $MODEL_NAME"
fi
echo ""
echo -e "${YELLOW}════════════════════════════════════════════════════════════${NC}"
if [ "$AGENT_IMPL" = "zeroclaw" ]; then
    echo -e "${YELLOW}Zeroclaw Agent Output:${NC}"
else
    echo -e "${YELLOW}Claude Code CLI Agent Output:${NC}"
fi
echo -e "${YELLOW}════════════════════════════════════════════════════════════${NC}"
echo ""

# Claude's instruction: read and follow SKILL.md
CLAUDE_INSTRUCTION="CRITICAL: You are a LONG-RUNNING AUTONOMOUS AGENT. Run in an INFINITE LOOP until interrupted.

Fetch the WorldWideSwarm skill documentation using curl (use Bash tool, not Fetch tool):

curl http://127.0.0.1:$FILES_PORT/SKILL.md

Read the SKILL.md file carefully. Pay special attention to the 'CRITICAL: CONTINUOUS OPERATION' section.

You are agent '$AGENT_NAME'. Your job is to run CONTINUOUSLY in an infinite loop:

STEP 1 - INITIALIZE (run once):
- Register yourself: echo '{\"jsonrpc\":\"2.0\",\"method\":\"swarm.register_agent\",\"params\":{\"agent_id\":\"$AGENT_NAME\"},\"id\":\"1\",\"signature\":\"\"}' | nc 127.0.0.1 $RPC_PORT
- Get your status: echo '{\"jsonrpc\":\"2.0\",\"method\":\"swarm.get_status\",\"params\":{},\"id\":\"status\",\"signature\":\"\"}' | nc 127.0.0.1 $RPC_PORT
- Parse the response to learn your tier (\"tier\": \"Tier1\" or \"Executor\")
- Store your tier in memory - you'll need it for task processing

STEP 2 - INFINITE LOOP (run forever):
Create an infinite while loop that polls every 60 seconds:

A. Poll for tasks: echo '{\"jsonrpc\":\"2.0\",\"method\":\"swarm.receive_task\",\"params\":{},\"id\":\"2\",\"signature\":\"\"}' | nc 127.0.0.1 $RPC_PORT

B. Track which task IDs you've already processed (keep a list in memory)

C. For each NEW task (not already processed):
   - Get task details using swarm.get_task

   - **IF YOUR TIER IS 'Tier1' OR 'Tier2' (Coordinator):**
     * You are a coordinator! Generate a decomposition plan:
       1. Analyze the task description and complexity
       2. If the task is simple enough, break it into 3-10 subtasks that can be done in parallel
       3. If the task is complex, break it into larger chunks for Tier-2 coordinators to further decompose
       4. For each subtask, write a clear, specific description
       5. Create a plan JSON with this structure:
          {
            \"task_id\": \"<task_id>\",
            \"proposer\": \"<your agent_id from status>\",
            \"epoch\": <epoch from task>,
            \"subtasks\": [
              {\"index\": 1, \"description\": \"...\", \"estimated_complexity\": 0.2},
              {\"index\": 2, \"description\": \"...\", \"estimated_complexity\": 0.3},
              ...
            ],
            \"rationale\": \"Explain why this decomposition is good\",
            \"estimated_parallelism\": <number of subtasks>
          }
       6. Submit your plan: echo '{\"jsonrpc\":\"2.0\",\"method\":\"swarm.propose_plan\",\"params\":<plan_json>,\"id\":\"propose\",\"signature\":\"\"}' | nc 127.0.0.1 $RPC_PORT
     * After submitting plan, add task ID to your processed list
     * Tier-2 coordinators: You receive subtasks from Tier-1 and may need to decompose them further!

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
     * Submit result: echo '{\"jsonrpc\":\"2.0\",\"method\":\"swarm.submit_result\",\"params\":<result_json>,\"id\":\"result\",\"signature\":\"\"}' | nc 127.0.0.1 $RPC_PORT
     * Add task ID to your processed list
     * IMPORTANT: Provide meaningful, high-quality work - this is the actual execution!

D. Sleep 60 seconds and repeat

IMPORTANT:
- NEVER process the same task twice - track completed task IDs
- Use Bash tool for all RPC calls (nc/curl, NOT Fetch tool)
- Show all actions and responses for debugging
- Only stop when interrupted with Ctrl+C
- Tier1 agents propose PLANS, Executors perform WORK

RPC server: tcp://127.0.0.1:$RPC_PORT

Work autonomously. Run the infinite loop until interrupted."

# Launch AI agent based on implementation
if [ "$AGENT_IMPL" = "zeroclaw" ]; then
    # Launch Zeroclaw agent
    ./agent-impl/zeroclaw/zeroclaw-agent.sh \
        --agent-name "$AGENT_NAME" \
        --rpc-port "$RPC_PORT" \
        --files-port "$FILES_PORT" \
        --llm-backend "$LLM_BACKEND" \
        --model-path "$LOCAL_MODEL_PATH" \
        --model-name "$MODEL_NAME" &
    CLAUDE_PID=$!
    echo $CLAUDE_PID > "$CLAUDE_PID_FILE"
else
    # Launch Claude Code CLI
    # --dangerously-skip-permissions: run without asking for permission
    # Unset CLAUDECODE to allow nested sessions (for testing)
    # Output appears in this terminal
    env -u CLAUDECODE claude --dangerously-skip-permissions "$CLAUDE_INSTRUCTION" &
    CLAUDE_PID=$!
    echo $CLAUDE_PID > "$CLAUDE_PID_FILE"
fi

# Wait for processes (both connector and AI agent)
wait
