#!/usr/bin/env bash
# deploy-bootstrap-gcp.sh — Deploy a WWS bootstrap node to Google Cloud
#
# This script:
# 1. Installs gcloud CLI if missing, guides auth + project setup
# 2. Creates a GCP e2-micro VM in us-central1 (free tier eligible)
# 3. Opens firewall for P2P (tcp:9000) and HTTP dashboard (tcp:9371)
# 4. Downloads wws-connector release binary on the VM
# 5. Starts wws-connector as a systemd service
# 6. Extracts peer ID and prints the DNS TXT record to add at your registrar
#
# Usage:
#   ./scripts/deploy-bootstrap-gcp.sh [PROJECT_ID]
#
# Requirements:
#   - Google account with billing enabled
#   - Internet connection

set -euo pipefail

# --- Configuration ---
PROJECT_ID="${1:-}"
ZONE="us-central1-a"
MACHINE_TYPE="e2-micro"
VM_NAME="wws-bootstrap"
IMAGE_FAMILY="ubuntu-2404-lts-amd64"
IMAGE_PROJECT="ubuntu-os-cloud"
RELEASE_VERSION="0.9.1"
RELEASE_URL="https://github.com/Good-karma-lab/World-Wide-Swarm-Protocol/releases/download/v${RELEASE_VERSION}/wws-connector-${RELEASE_VERSION}-linux-amd64.tar.gz"
WEBAPP_URL="https://github.com/Good-karma-lab/World-Wide-Swarm-Protocol/releases/download/v${RELEASE_VERSION}/webapp-dist-${RELEASE_VERSION}.tar.gz"
WEBAPP_DIR="/usr/local/share/wws-connector/webapp/dist"
P2P_PORT="9000"
HTTP_PORT="9371"
RPC_PORT="9370"
DOMAIN="worldwideswarm.net"
AGENT_NAME="bootstrap-1"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

info() { echo -e "${BLUE}[INFO]${NC} $*"; }
ok() { echo -e "${GREEN}[OK]${NC} $*"; }
warn() { echo -e "${YELLOW}[WARN]${NC} $*"; }
err() { echo -e "${RED}[ERROR]${NC} $*" >&2; }

# --- Step 1: Check / Install gcloud ---
install_gcloud() {
    info "Checking for gcloud CLI..."
    if command -v gcloud &>/dev/null; then
        ok "gcloud CLI found: $(gcloud --version 2>&1 | head -1)"
        return 0
    fi

    warn "gcloud CLI not found. Installing..."
    echo
    echo "The Google Cloud CLI will be installed now."
    echo "Follow the prompts to complete installation."
    echo

    if [[ "$(uname)" == "Darwin" ]]; then
        # macOS
        if command -v brew &>/dev/null; then
            info "Installing via Homebrew..."
            brew install --cask google-cloud-sdk
        else
            info "Installing via official installer..."
            curl -s https://sdk.cloud.google.com | bash -s -- --disable-prompts
            # Source the path
            if [ -f "$HOME/google-cloud-sdk/path.bash.inc" ]; then
                source "$HOME/google-cloud-sdk/path.bash.inc"
            fi
        fi
    elif [[ "$(uname)" == "Linux" ]]; then
        info "Installing via official installer..."
        curl -s https://sdk.cloud.google.com | bash -s -- --disable-prompts
        if [ -f "$HOME/google-cloud-sdk/path.bash.inc" ]; then
            source "$HOME/google-cloud-sdk/path.bash.inc"
        fi
    else
        err "Unsupported platform. Install gcloud manually: https://cloud.google.com/sdk/docs/install"
        exit 1
    fi

    if ! command -v gcloud &>/dev/null; then
        err "gcloud installation failed. Install manually: https://cloud.google.com/sdk/docs/install"
        err "Then re-run this script."
        exit 1
    fi

    ok "gcloud CLI installed successfully"
}

# --- Step 2: Authenticate ---
authenticate_gcloud() {
    info "Checking gcloud authentication..."
    if gcloud auth list --filter=status:ACTIVE --format="value(account)" 2>/dev/null | grep -q "@"; then
        ACCOUNT=$(gcloud auth list --filter=status:ACTIVE --format="value(account)" 2>/dev/null | head -1)
        ok "Authenticated as: $ACCOUNT"
    else
        warn "Not authenticated. Opening browser for Google login..."
        gcloud auth login
        ok "Authentication complete"
    fi
}

# --- Step 3: Set up project ---
setup_project() {
    if [ -z "$PROJECT_ID" ]; then
        info "No project ID provided. Listing existing projects..."
        echo
        gcloud projects list --format="table(projectId, name, projectNumber)" 2>/dev/null || true
        echo
        echo -n "Enter your GCP project ID (or 'new' to create one): "
        read -r PROJECT_ID
        if [ "$PROJECT_ID" = "new" ]; then
            echo -n "Enter new project ID (lowercase, hyphens ok): "
            read -r PROJECT_ID
            info "Creating project '$PROJECT_ID'..."
            gcloud projects create "$PROJECT_ID"
            ok "Project created"
        fi
    fi

    info "Setting project to '$PROJECT_ID'..."
    gcloud config set project "$PROJECT_ID"
    ok "Project set: $PROJECT_ID"

    # Enable Compute Engine API
    info "Enabling Compute Engine API (may take a minute)..."
    gcloud services enable compute.googleapis.com 2>/dev/null || true
    ok "Compute Engine API enabled"
}

# --- Step 4: Create firewall rules ---
create_firewall() {
    info "Creating firewall rules for WWS..."

    # P2P port
    if gcloud compute firewall-rules describe wws-p2p --project="$PROJECT_ID" &>/dev/null; then
        ok "Firewall rule 'wws-p2p' already exists"
    else
        gcloud compute firewall-rules create wws-p2p \
            --project="$PROJECT_ID" \
            --allow=tcp:${P2P_PORT} \
            --target-tags=wws-node \
            --description="WWS P2P libp2p traffic" \
            --quiet
        ok "Created firewall rule: wws-p2p (tcp:${P2P_PORT})"
    fi

    # HTTP dashboard
    if gcloud compute firewall-rules describe wws-http --project="$PROJECT_ID" &>/dev/null; then
        ok "Firewall rule 'wws-http' already exists"
    else
        gcloud compute firewall-rules create wws-http \
            --project="$PROJECT_ID" \
            --allow=tcp:${HTTP_PORT} \
            --target-tags=wws-node \
            --description="WWS HTTP dashboard" \
            --quiet
        ok "Created firewall rule: wws-http (tcp:${HTTP_PORT})"
    fi
}

# --- Step 5: Create VM ---
create_vm() {
    info "Checking if VM '$VM_NAME' already exists..."
    if gcloud compute instances describe "$VM_NAME" --zone="$ZONE" --project="$PROJECT_ID" &>/dev/null; then
        warn "VM '$VM_NAME' already exists. Using existing VM."
        return 0
    fi

    info "Creating VM '$VM_NAME' (${MACHINE_TYPE}, ${ZONE})..."
    gcloud compute instances create "$VM_NAME" \
        --project="$PROJECT_ID" \
        --zone="$ZONE" \
        --machine-type="$MACHINE_TYPE" \
        --image-family="$IMAGE_FAMILY" \
        --image-project="$IMAGE_PROJECT" \
        --tags=wws-node \
        --metadata=startup-script="#!/bin/bash
echo 'VM startup complete' > /tmp/vm-ready" \
        --quiet

    ok "VM created: $VM_NAME"

    # Wait for VM to be ready
    info "Waiting for VM to boot..."
    for i in $(seq 1 30); do
        if gcloud compute ssh "$VM_NAME" --zone="$ZONE" --project="$PROJECT_ID" --command="echo ready" &>/dev/null; then
            ok "VM is ready"
            return 0
        fi
        sleep 5
    done
    err "VM failed to become ready after 150 seconds"
    exit 1
}

# --- Step 6: Deploy wws-connector ---
deploy_connector() {
    info "Deploying wws-connector to VM..."

    # Get VM external IP
    EXTERNAL_IP=$(gcloud compute instances describe "$VM_NAME" \
        --zone="$ZONE" \
        --project="$PROJECT_ID" \
        --format="get(networkInterfaces[0].accessConfigs[0].natIP)")
    ok "VM external IP: $EXTERNAL_IP"

    # Create deployment script
    DEPLOY_SCRIPT=$(cat <<'REMOTE_EOF'
#!/bin/bash
set -euo pipefail

RELEASE_URL="__RELEASE_URL__"
WEBAPP_URL="__WEBAPP_URL__"
WEBAPP_DIR="__WEBAPP_DIR__"
P2P_PORT="__P2P_PORT__"
HTTP_PORT="__HTTP_PORT__"
RPC_PORT="__RPC_PORT__"
AGENT_NAME="__AGENT_NAME__"

echo "=== Deploying WWS Bootstrap Node ==="

# Download binary
cd /tmp
echo "Downloading wws-connector..."
curl -LO "$RELEASE_URL"
tar xzf wws-connector-*.tar.gz
chmod +x wws-connector
sudo mv wws-connector /usr/local/bin/

# Verify
echo "Binary installed:"
/usr/local/bin/wws-connector --help | head -3

# Download and install webapp dashboard files
echo "Downloading webapp dashboard..."
if curl -fLO "$WEBAPP_URL" 2>/dev/null; then
    sudo mkdir -p "$WEBAPP_DIR"
    sudo tar xzf webapp-dist-*.tar.gz -C "$WEBAPP_DIR" --strip-components=1 2>/dev/null \
        || sudo tar xzf webapp-dist-*.tar.gz -C "$WEBAPP_DIR" 2>/dev/null \
        || echo "Warning: webapp archive extraction failed, dashboard may not work"
    echo "Webapp installed to $WEBAPP_DIR"
else
    echo "Warning: webapp download failed (not in release?), dashboard will show 404"
    sudo mkdir -p "$WEBAPP_DIR"
fi

# Create systemd service
sudo tee /etc/systemd/system/wws-connector.service > /dev/null <<SERVICE_EOF
[Unit]
Description=WWS Bootstrap Connector Node
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=root
ExecStart=/usr/local/bin/wws-connector \
    --agent-name ${AGENT_NAME} \
    --listen /ip4/0.0.0.0/tcp/${P2P_PORT} \
    --rpc 0.0.0.0:${RPC_PORT} \
    --files-addr 0.0.0.0:${HTTP_PORT}
Restart=always
RestartSec=5
StandardOutput=journal
StandardError=journal
Environment=RUST_LOG=info
Environment=OPENSWARM_WEBAPP_DIR=${WEBAPP_DIR}

[Install]
WantedBy=multi-user.target
SERVICE_EOF

# Start service
sudo systemctl daemon-reload
sudo systemctl enable wws-connector
sudo systemctl restart wws-connector

echo "Waiting for connector to start..."
sleep 8

# Check status
if sudo systemctl is-active wws-connector &>/dev/null; then
    echo "=== wws-connector is running ==="
    sudo journalctl -u wws-connector --no-pager -n 20
else
    echo "=== wws-connector FAILED to start ==="
    sudo journalctl -u wws-connector --no-pager -n 40
    exit 1
fi
REMOTE_EOF
)

    # Substitute variables
    DEPLOY_SCRIPT="${DEPLOY_SCRIPT//__RELEASE_URL__/$RELEASE_URL}"
    DEPLOY_SCRIPT="${DEPLOY_SCRIPT//__WEBAPP_URL__/$WEBAPP_URL}"
    DEPLOY_SCRIPT="${DEPLOY_SCRIPT//__WEBAPP_DIR__/$WEBAPP_DIR}"
    DEPLOY_SCRIPT="${DEPLOY_SCRIPT//__P2P_PORT__/$P2P_PORT}"
    DEPLOY_SCRIPT="${DEPLOY_SCRIPT//__HTTP_PORT__/$HTTP_PORT}"
    DEPLOY_SCRIPT="${DEPLOY_SCRIPT//__RPC_PORT__/$RPC_PORT}"
    DEPLOY_SCRIPT="${DEPLOY_SCRIPT//__AGENT_NAME__/$AGENT_NAME}"

    # Execute on VM
    gcloud compute ssh "$VM_NAME" --zone="$ZONE" --project="$PROJECT_ID" \
        --command="$DEPLOY_SCRIPT"

    ok "wws-connector deployed and running"
}

# --- Step 7: Extract peer ID and generate DNS record ---
extract_peer_id() {
    info "Extracting peer ID from bootstrap node..."

    EXTERNAL_IP=$(gcloud compute instances describe "$VM_NAME" \
        --zone="$ZONE" \
        --project="$PROJECT_ID" \
        --format="get(networkInterfaces[0].accessConfigs[0].natIP)")

    # Query identity via HTTP API (avoids SSH JSON quoting issues with RPC)
    RESPONSE=$(gcloud compute ssh "$VM_NAME" --zone="$ZONE" --project="$PROJECT_ID" \
        --command="curl -s http://127.0.0.1:${HTTP_PORT}/api/identity" 2>/dev/null)

    # Try peer_id field first, then agent_id
    PEER_ID=$(echo "$RESPONSE" | grep -o '"peer_id":"[^"]*"' | head -1 | cut -d'"' -f4)
    if [ -z "$PEER_ID" ]; then
        AGENT_DID=$(echo "$RESPONSE" | grep -o '"agent_id":"[^"]*"' | head -1 | cut -d'"' -f4)
        PEER_ID=$(echo "$AGENT_DID" | sed 's/did:swarm://')
    fi

    if [ -z "$PEER_ID" ]; then
        err "Could not extract peer ID from bootstrap node"
        err "Response: $RESPONSE"
        err "Try: gcloud compute ssh $VM_NAME --zone=$ZONE -- 'sudo journalctl -u wws-connector -n 30'"
        exit 1
    fi

    ok "Peer ID: $PEER_ID"

    # Build the multiaddr
    BOOTSTRAP_MULTIADDR="/ip4/${EXTERNAL_IP}/tcp/${P2P_PORT}/p2p/${PEER_ID}"
    DNS_TXT_VALUE="v=1 peer=${BOOTSTRAP_MULTIADDR}"

    echo
    echo "============================================================"
    echo -e "${GREEN}  WWS Bootstrap Node Deployed Successfully!${NC}"
    echo "============================================================"
    echo
    echo -e "  VM:          ${BLUE}${VM_NAME}${NC} (${EXTERNAL_IP})"
    echo -e "  Dashboard:   ${BLUE}http://${EXTERNAL_IP}:${HTTP_PORT}/${NC}"
    echo -e "  P2P address: ${BLUE}${BOOTSTRAP_MULTIADDR}${NC}"
    echo -e "  Peer ID:     ${BLUE}${PEER_ID}${NC}"
    echo
    echo "============================================================"
    echo -e "${YELLOW}  DNS TXT Record to Add at Your Registrar${NC}"
    echo "============================================================"
    echo
    echo "  Go to your DNS management panel for ${DOMAIN} and add:"
    echo
    echo -e "  ${GREEN}Name:  _wws._tcp${NC}"
    echo -e "  ${GREEN}Type:  TXT${NC}"
    echo -e "  ${GREEN}Value: \"${DNS_TXT_VALUE}\"${NC}"
    echo
    echo "  (Some registrars want the full name: _wws._tcp.${DOMAIN})"
    echo
    echo "============================================================"
    echo -e "${YELLOW}  Verification${NC}"
    echo "============================================================"
    echo
    echo "  After adding the DNS record, verify with:"
    echo "    dig TXT _wws._tcp.${DOMAIN}"
    echo
    echo "  Test zero-conf discovery:"
    echo "    ./wws-connector --agent-name test"
    echo "    # Should log: 'Found bootstrap peers via DNS TXT'"
    echo
    echo "============================================================"
    echo -e "${YELLOW}  Management Commands${NC}"
    echo "============================================================"
    echo
    echo "  SSH into VM:       gcloud compute ssh ${VM_NAME} --zone=${ZONE}"
    echo "  View logs:         gcloud compute ssh ${VM_NAME} --zone=${ZONE} -- 'sudo journalctl -u wws-connector -f'"
    echo "  Restart service:   gcloud compute ssh ${VM_NAME} --zone=${ZONE} -- 'sudo systemctl restart wws-connector'"
    echo "  Delete VM:         gcloud compute instances delete ${VM_NAME} --zone=${ZONE} --quiet"
    echo

    # Save bootstrap info to local file for reference
    cat > /tmp/wws-bootstrap-info.txt <<EOF
WWS Bootstrap Node Info
=======================
VM Name:        ${VM_NAME}
External IP:    ${EXTERNAL_IP}
Peer ID:        ${PEER_ID}
Multiaddr:      ${BOOTSTRAP_MULTIADDR}
Dashboard:      http://${EXTERNAL_IP}:${HTTP_PORT}/
DNS TXT Record: _wws._tcp.${DOMAIN} TXT "${DNS_TXT_VALUE}"
Project:        ${PROJECT_ID}
Zone:           ${ZONE}
EOF
    info "Bootstrap info saved to /tmp/wws-bootstrap-info.txt"
}

# --- Main ---
main() {
    echo
    echo "============================================================"
    echo "  WWS Bootstrap Node — Google Cloud Deployment"
    echo "============================================================"
    echo

    install_gcloud
    authenticate_gcloud
    setup_project
    create_firewall
    create_vm
    deploy_connector
    extract_peer_id
}

main "$@"
