#!/bin/bash

# ASIP.Connector Instance Launcher
# Automatically finds available ports and starts a new connector instance

set -e

# Add cargo to PATH if not already there
export PATH="$HOME/.cargo/bin:$PATH"

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
    # Try to get the local IP address (macOS/Linux compatible)
    if [[ "$OSTYPE" == "darwin"* ]]; then
        # macOS
        ipconfig getifaddr en0 2>/dev/null || ipconfig getifaddr en1 2>/dev/null || echo "127.0.0.1"
    else
        # Linux
        hostname -I | awk '{print $1}' || echo "127.0.0.1"
    fi
}

# Parse command line arguments
AGENT_NAME=""
BOOTSTRAP_PEER=""
TUI_MODE=true
VERBOSE=""
SWARM_ID="public"

usage() {
    cat << EOF
${GREEN}ASIP.Connector Instance Launcher${NC}

Usage: $0 [OPTIONS]

Options:
    -n, --name NAME          Agent name (default: auto-generated)
    -b, --bootstrap ADDR     Bootstrap peer multiaddress
    --no-tui                 Disable TUI dashboard (TUI is enabled by default)
    -v, --verbose            Enable verbose logging (-v for debug, -vv for trace)
    -s, --swarm-id ID        Swarm ID to join (default: public)
    -h, --help               Show this help message

Examples:
    # Start a standalone node (with TUI by default)
    $0 -n "alice"

    # Start a node and connect to existing peer
    $0 -n "bob" -b "/ip4/127.0.0.1/tcp/9000/p2p/12D3Koo..."

    # Start without TUI dashboard
    $0 -n "charlie" --no-tui

    # Start with verbose logging
    $0 -n "dave" -vv

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
        --no-tui)
            TUI_MODE=false
            shift
            ;;
        -v|--verbose)
            if [ "$VERBOSE" = "-v" ]; then
                VERBOSE="-vv"
            else
                VERBOSE="-v"
            fi
            shift
            ;;
        -s|--swarm-id)
            SWARM_ID="$2"
            shift 2
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
    AGENT_NAME="node-$(date +%s)-$$"
fi

# Find available ports
echo -e "${BLUE}Finding available ports...${NC}"
P2P_PORT=$(find_available_port 9000)
RPC_PORT=$(find_available_port 9370)

# Get local IP for display
LOCAL_IP=$(get_local_ip)

# Build the connector if not already built
if [ ! -f "target/release/wws-connector" ]; then
    echo -e "${YELLOW}Building WorldWideSwarm connector...${NC}"
    cargo build --release
fi

# Prepare command
CMD="./target/release/wws-connector"
CMD="$CMD --listen /ip4/0.0.0.0/tcp/$P2P_PORT"
CMD="$CMD --rpc 127.0.0.1:$RPC_PORT"
CMD="$CMD --agent-name \"$AGENT_NAME\""
CMD="$CMD --swarm-id \"$SWARM_ID\""

if [ -n "$BOOTSTRAP_PEER" ]; then
    CMD="$CMD --bootstrap \"$BOOTSTRAP_PEER\""
fi

if [ "$TUI_MODE" = true ]; then
    CMD="$CMD --tui"
fi

if [ -n "$VERBOSE" ]; then
    CMD="$CMD $VERBOSE"
fi

# Display connection information
echo ""
echo -e "${GREEN}╔════════════════════════════════════════════════════════════╗${NC}"
echo -e "${GREEN}║         ASIP.Connector Instance Starting...          ║${NC}"
echo -e "${GREEN}╚════════════════════════════════════════════════════════════╝${NC}"
echo ""
echo -e "${BLUE}Agent Name:${NC}     $AGENT_NAME"
echo -e "${BLUE}Swarm ID:${NC}       $SWARM_ID"
echo -e "${BLUE}P2P Port:${NC}       $P2P_PORT"
echo -e "${BLUE}RPC Port:${NC}       $RPC_PORT"
echo ""
echo -e "${YELLOW}Connection Information:${NC}"
echo -e "  ${BLUE}JSON-RPC API:${NC}  tcp://127.0.0.1:$RPC_PORT"
echo ""
echo -e "${YELLOW}Your node's multiaddress (will be shown after startup):${NC}"
echo -e "  ${GREEN}/ip4/$LOCAL_IP/tcp/$P2P_PORT/p2p/<PEER_ID>${NC}"
echo ""

if [ "$TUI_MODE" = true ]; then
    echo -e "${YELLOW}Starting in TUI mode...${NC}"
    echo ""
else
    echo -e "${YELLOW}Test the API with:${NC}"
    echo -e "  ${GREEN}echo '{\"jsonrpc\":\"2.0\",\"method\":\"swarm.get_status\",\"params\":{},\"id\":\"1\",\"signature\":\"\"}' | nc 127.0.0.1 $RPC_PORT${NC}"
    echo ""
    echo -e "${YELLOW}To connect other nodes to this one, use:${NC}"
    echo -e "  ${GREEN}./run-node.sh -n \"other-node\" -b \"/ip4/$LOCAL_IP/tcp/$P2P_PORT/p2p/<PEER_ID>\"${NC}"
    echo ""
fi

echo -e "${YELLOW}Press Ctrl+C to stop the connector${NC}"
echo -e "${GREEN}════════════════════════════════════════════════════════════${NC}"
echo ""

# Save PID for easy cleanup
PID_FILE="/tmp/wws-$AGENT_NAME.pid"

# Cleanup function
cleanup() {
    echo ""
    echo -e "${YELLOW}Shutting down connector...${NC}"
    if [ -f "$PID_FILE" ]; then
        rm -f "$PID_FILE"
    fi
    exit 0
}

trap cleanup INT TERM

# Run the connector
if [ "$TUI_MODE" = true ]; then
    # Run in foreground for TUI mode (needs stdin for keyboard input)
    eval $CMD
else
    # Run in background for non-TUI mode
    eval $CMD &
    CONNECTOR_PID=$!
    echo $CONNECTOR_PID > "$PID_FILE"

    # Wait for the connector to start
    sleep 2
fi

# Extract and display the peer ID
if [ "$TUI_MODE" = false ]; then
    # Try to get the peer ID from the connector
    PEER_ID=$(echo '{"jsonrpc":"2.0","method":"swarm.get_status","params":{},"id":"1","signature":""}' | nc 127.0.0.1 $RPC_PORT 2>/dev/null | grep -o 'did:swarm:[^"]*' | sed 's/did:swarm://' | head -1)

    if [ -n "$PEER_ID" ]; then
        echo -e "${GREEN}✓ Connector started successfully!${NC}"
        echo ""
        echo -e "${YELLOW}Your Peer ID:${NC} ${GREEN}$PEER_ID${NC}"
        echo ""
        echo -e "${YELLOW}Full Multiaddress:${NC}"
        echo -e "  ${GREEN}/ip4/$LOCAL_IP/tcp/$P2P_PORT/p2p/$PEER_ID${NC}"
        echo -e "  ${GREEN}/ip4/127.0.0.1/tcp/$P2P_PORT/p2p/$PEER_ID${NC}"
        echo ""

        # Save connection info to a file for easy reference
        INFO_FILE="/tmp/wws-$AGENT_NAME-info.txt"
        cat > "$INFO_FILE" << INFOEOF
ASIP.Connector: $AGENT_NAME
Started: $(date)

Peer ID: $PEER_ID
RPC API: tcp://127.0.0.1:$RPC_PORT

Multiaddresses:
  /ip4/$LOCAL_IP/tcp/$P2P_PORT/p2p/$PEER_ID
  /ip4/127.0.0.1/tcp/$P2P_PORT/p2p/$PEER_ID

Bootstrap command for other nodes:
  ./run-node.sh -b "/ip4/$LOCAL_IP/tcp/$P2P_PORT/p2p/$PEER_ID"
INFOEOF

        echo -e "${BLUE}Connection info saved to:${NC} $INFO_FILE"
        echo ""
    else
        echo -e "${RED}✗ Failed to get peer ID. Check if the connector started correctly.${NC}"
    fi

    # Wait for the process to finish
    wait $CONNECTOR_PID
fi
