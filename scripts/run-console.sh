#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

export PATH="$HOME/.cargo/bin:$PATH"

agent_name_set=false
for arg in "$@"; do
    if [[ "$arg" == "--agent-name" ]]; then
        agent_name_set=true
        break
    fi
done

echo "Rebuilding WorldWideSwarm connector..."
cargo build --release

cmd=("./target/release/wws-connector" "--console")

if [[ "$agent_name_set" == false ]]; then
    cmd+=("--agent-name" "operator")
fi

cmd+=("$@")

echo "Starting Operator Console..."
exec "${cmd[@]}"
