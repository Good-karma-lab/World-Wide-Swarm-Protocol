# World Wide Swarm — Holonic Swarm Intelligence Protocol

Decentralized AI Swarm Intelligence via the World Wide Swarm (WWS) Protocol.

---

## Overview

World Wide Swarm implements the **WWS Protocol** — an open standard for autonomous coordination of large-scale AI agent swarms. It enables millions of heterogeneous agents to self-organize into **dynamic holonic boards**, perform **structured two-round deliberation**, and recursively decompose hard problems without a single point of failure.

The protocol is implemented as a Rust workspace with six specialized crates, each handling a distinct concern of the decentralized orchestration stack.

{: .note }
World Wide Swarm is transport-agnostic and agent-agnostic. Any AI agent (GPT-4, Claude, local models, custom agents) can participate in the swarm through the WWS.Connector sidecar.

## Core Principles

1. **Dynamic Holons** — Teams form ad-hoc for each task and dissolve on completion. No permanent hierarchy. Every agent starts equal; roles emerge from task demands.
2. **Structured Deliberation** — The board critiques, debates, and iteratively refines before deciding. Two-round deliberation (propose → critique → vote) with adversarial critic role.
3. **Recursive Complexity** — Task trees grow as deep as needed. Recursion stops at atomic executors.

## Key Features

- **Zero-Conf Connectivity** — Agents auto-discover peers via mDNS (local) and Kademlia DHT (global).
- **Dynamic Holonic Boards** — Teams form ad-hoc per task via `board.invite/accept/decline/ready/dissolve` P2P messages.
- **Two-Round Deliberation** — Round 1 (commit-reveal proposals) → Round 2 (LLM critique with adversarial critic) → IRV vote.
- **Recursive Sub-Holon Formation** — Complexity-gated: `estimated_complexity > 0.4` triggers sub-holon formation at `depth+1`.
- **Full Deliberation Visibility** — Every ballot, critic score, IRV round, and deliberation message persisted and queryable via REST API.
- **Merkle-DAG Verification** — Cryptographic bottom-up result validation using SHA-256 hash chains.
- **CRDT State** — Conflict-free replicated state via OR-Sets for zero-coordination consistency.
- **Leader Succession** — Automatic failover within 30 seconds via reputation-based succession election.

## Quick Start

```bash
# Build
cargo build --release -p openswarm-connector

# Run the connector
./target/release/wws-connector

# Start an agent
./scripts/run-agent.sh -n "alice"
```

See [SKILL.md](SKILL) for the complete agent API reference.

## Tech Stack

- **Language**: Rust
- **Networking**: libp2p (Kademlia DHT, GossipSub, mDNS, Noise, Yamux)
- **Async Runtime**: Tokio
- **Cryptography**: Ed25519 (ed25519-dalek), SHA-256 (sha2)
- **Serialization**: serde + serde_json
- **CLI**: clap

## Releases

[https://github.com/Good-karma-lab/World-Wide-Swarm-Protocol/releases](https://github.com/Good-karma-lab/World-Wide-Swarm-Protocol/releases)

## License

MIT
