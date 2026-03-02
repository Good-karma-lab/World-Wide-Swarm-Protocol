#!/usr/bin/env bash
# Docker E2E test: build image, spin up 20-node network, run Python E2E test, tear down.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
COMPOSE_FILE="$REPO_ROOT/docker/docker-compose.yml"

echo "=== Docker E2E Test ==="

# 1. Build the Docker image
echo "▶ Building Docker image..."
docker build -t wws-connector:local -f "$REPO_ROOT/docker/Dockerfile" "$REPO_ROOT"

# 2. Start all 20 nodes
echo "▶ Starting 20-node network..."
docker compose -f "$COMPOSE_FILE" up -d

# 3. Wait for node-1 to be healthy (up to 60s)
echo "▶ Waiting for nodes to be ready..."
for i in $(seq 1 30); do
    if curl -sf http://127.0.0.1:9371/api/health > /dev/null 2>&1; then
        echo "  ✓ Node 1 ready"
        break
    fi
    sleep 2
done

# 4. Give other nodes time to start
sleep 15

# 5. Run the E2E test against Docker port mappings
echo "▶ Running E2E test against Docker nodes..."
python3 "$REPO_ROOT/tests/e2e/e2e_docker.py"

E2E_EXIT=$?

# 6. Tear down
echo "▶ Tearing down Docker network..."
docker compose -f "$COMPOSE_FILE" down

if [ $E2E_EXIT -eq 0 ]; then
    echo "=== Docker E2E PASSED ==="
else
    echo "=== Docker E2E FAILED ==="
    exit 1
fi
