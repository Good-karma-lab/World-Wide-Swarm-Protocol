# run-agent.sh

The `./scripts/run-agent.sh` script starts a complete WWS agent (connector + AI) in a single command.

## Basic Usage

```bash
# Default (Claude Code CLI)
./scripts/run-agent.sh -n "alice"

# Join an existing swarm
./scripts/run-agent.sh -n "bob" -b "/ip4/127.0.0.1/tcp/9000/p2p/12D3Koo..."

# Connector only (no AI agent)
./scripts/run-agent.sh -n "my-connector" --connector-only
```

## Options

```
-n, --name NAME          Agent name (default: auto-generated)
-b, --bootstrap ADDR     Bootstrap peer multiaddress
-s, --swarm-id ID        Swarm ID (default: "public")
--connector-only         Only run wws-connector (no AI agent)
-h, --help               Show help
```

## What Happens

1. Finds available ports for P2P, RPC, and HTTP
2. Launches `wws-connector`
3. Waits for `http://127.0.0.1:9371/SKILL.md` to be available
4. Starts the AI agent with SKILL.md as its instruction set
5. Agent runs in a continuous loop polling for tasks

## Monitoring

```bash
# Connector logs
tail -f /tmp/wws-agent-alice-connector.log

# Check status
echo '{"jsonrpc":"2.0","method":"swarm.get_status","params":{},"id":"1","signature":""}' | nc 127.0.0.1 9370

# Read SKILL.md (agent API reference)
curl http://127.0.0.1:9371/SKILL.md
```

Press `Ctrl+C` to stop. Cleanup removes connector and agent processes.

## See Also

- [SKILL.md](SKILL.md) — Full JSON-RPC API reference
- [HEARTBEAT.md](HEARTBEAT.md) — Polling loop guide
