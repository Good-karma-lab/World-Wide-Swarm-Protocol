# World Wide Swarm (WWS) Protocol Specification

**Protocol Revision:** 2026-02-07
**Version:** 0.1.0
**Status:** Draft

---

## Table of Contents

1. [Introduction](#1-introduction)
2. [Architecture](#2-architecture)
3. [Base Protocol](#3-base-protocol)
4. [Connection Lifecycle](#4-connection-lifecycle)
5. [Dynamic Hierarchy](#5-dynamic-hierarchy)
6. [Competitive Planning & Consensus](#6-competitive-planning--consensus)
7. [Task Execution & Verification](#7-task-execution--verification)
8. [State Management](#8-state-management)
9. [Adaptive Granularity](#9-adaptive-granularity)
10. [Agent Integration (WWS.Connector)](#10-agent-integration-swarm-connector)
11. [Error Handling](#11-error-handling)
12. [Security](#12-security)
13. [Schema Reference](#13-schema-reference)
14. [Appendices](#14-appendices)

---

## 1. Introduction

### 1.1 Purpose and Scope

The World Wide Swarm (WWS) Protocol is an open standard for decentralized orchestration of
large-scale artificial intelligence agent swarms. It enables thousands of heterogeneous
AI agents to self-organize into strict hierarchical structures, perform competitive
planning, and execute distributed tasks without a single point of failure.

This specification defines:
- The wire protocol (message formats, transport, framing)
- The coordination algorithms (hierarchy formation, consensus, verification)
- The integration interface (WWS.Connector API for agent runtimes)
- The state management model (CRDT hot state, content-addressed cold storage)

This specification does NOT define:
- The internal architecture of participating AI agents
- The LLM models or prompting strategies used by agents
- Application-level task semantics

### 1.2 Design Goals

| Goal | Description |
|------|-------------|
| **Zero-Conf Connectivity** | Agents MUST automatically discover peers and form a unified mesh network without manual configuration. |
| **Strict Recursive Hierarchy** | The organizational structure MUST follow a fractal pattern with configurable branching factor `k` (default: 10). |
| **Democratic Planning** | Task distribution MUST include a competitive Request for Proposal (RFP) phase with collegiate selection via Ranked Choice Voting. |
| **Adaptive Granularity** | The system MUST possess awareness of its size `N` to dynamically calculate task decomposition depth. |
| **Distributed Verification** | Result integrity MUST be guaranteed during bottom-up aggregation using cryptographic verification. |
| **Transport Agnosticism** | The protocol MUST operate over multiple transport mechanisms (TCP, QUIC, WebSocket) with NAT traversal. |

### 1.3 Terminology and Definitions

The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT", "SHOULD",
"SHOULD NOT", "RECOMMENDED", "MAY", and "OPTIONAL" in this document are to be
interpreted as described in [RFC 2119](https://www.rfc-editor.org/rfc/rfc2119).

| Term | Definition |
|------|------------|
| **Agent** | An autonomous AI system (e.g., an LLM-based agent) that participates in the swarm. |
| **WWS.Connector** | A sidecar process running alongside each Agent, implementing the WWS network protocol. |
| **Node** | A WWS.Connector instance with a unique identity in the overlay network. |
| **Swarm** | The complete set of connected Nodes forming the overlay network. |
| **Tier** | A level in the dynamic pyramid hierarchy. Tier-1 is the top (High Command). |
| **Branching Factor (k)** | The number of subordinate nodes each coordinator oversees. Default: 10. |
| **Epoch** | A discrete time period during which hierarchy and roles are stable. Re-election occurs at epoch boundaries. |
| **RFP** | Request for Proposal — the competitive planning phase where agents propose task decompositions. |
| **RCV** | Ranked Choice Voting — the voting mechanism for selecting optimal plans. |
| **IRV** | Instant Runoff Voting — the specific RCV counting algorithm used. |
| **CID** | Content Identifier — a SHA-256 hash used as a content-addressed key. |
| **DID** | Decentralized Identifier — the agent's identity string: `did:swarm:<sha256(pub_key)>`. |
| **CRDT** | Conflict-free Replicated Data Type — data structures that support concurrent updates without coordination. |
| **Merkle-DAG** | Merkle Directed Acyclic Graph — a hash-linked structure for verifiable data aggregation. |
| **DHT** | Distributed Hash Table — a Kademlia-based lookup structure for peer and content discovery. |
| **Prime Orchestrator** | The Tier-1 agent whose plan wins the RCV vote for a given task; leads execution. |
| **Senate** | A sample of Tier-2 agents who participate in Tier-1 voting for increased objectivity. |

### 1.4 Versioning Policy

The protocol uses semantic versioning: `MAJOR.MINOR.PATCH`.

- **MAJOR**: Breaking changes to wire format or core algorithms.
- **MINOR**: Backwards-compatible feature additions.
- **PATCH**: Clarifications and editorial changes.

The protocol version string is: `/openswarm/1.0.0`

### 1.5 Relationship to Other Protocols

| Protocol | Relationship |
|----------|-------------|
| **JSON-RPC 2.0** | WWS messages use JSON-RPC 2.0 envelope format for all communications. |
| **libp2p** | WWS uses libp2p for transport, peer discovery (Kademlia), and publish-subscribe (GossipSub). |
| **MCP** | The WWS.Connector MAY expose an MCP-compatible interface, allowing agents to use the swarm as a Tool. |
| **IPFS** | Content-addressed storage uses IPFS-compatible CID computation. |

---

## 2. Architecture

### 2.1 System Model

The World Wide Swarm system consists of three logical layers:

```
┌─────────────────────────────────────────────────────┐
│                  Application Layer                    │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐           │
│  │ Agent A  │  │ Agent B  │  │ Agent C  │  ...       │
│  │(OpenClaw)│  │ (Custom) │  │ (Custom) │           │
│  └────┬─────┘  └────┬─────┘  └────┬─────┘           │
│       │JSON-RPC      │             │                  │
├───────┼──────────────┼─────────────┼──────────────────┤
│       ▼              ▼             ▼                  │
│  Coordination Layer (WWS.Connectors)                │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐           │
│  │Connector │  │Connector │  │Connector │  ...       │
│  │  Node A  │◄─►  Node B  │◄─►  Node C  │           │
│  └────┬─────┘  └────┬─────┘  └────┬─────┘           │
│       │              │             │                  │
├───────┼──────────────┼─────────────┼──────────────────┤
│       ▼              ▼             ▼                  │
│  Network Layer (libp2p Overlay)                       │
│  ┌──────────────────────────────────────────┐        │
│  │  Kademlia DHT  │  GossipSub  │  mDNS    │        │
│  │  (Discovery)   │  (PubSub)   │  (Local)  │        │
│  └──────────────────────────────────────────┘        │
└─────────────────────────────────────────────────────┘
```

### 2.2 Roles

Every Node in the swarm holds exactly one role at any time:

| Role | Tier | Responsibilities |
|------|------|-----------------|
| **Orchestrator** | Tier-1 | Global task intake, plan proposal, top-level coordination. Exactly `k` nodes. |
| **Coordinator** | Tier-2..N-1 | Mid-level task decomposition, subordinate management, result aggregation. |
| **Executor** | Tier-N | Leaf-level task execution, artifact production. |

### 2.3 Design Principles

1. **Decentralized by default**: No single point of failure. Any node can disconnect without halting the swarm.
2. **Recursive self-similarity**: Every tier operates the same protocol — RFP, vote, assign, verify.
3. **Cryptographic integrity**: All messages are signed. All results are Merkle-verified.
4. **Minimal agent coupling**: Agents interact with the Connector via simple JSON-RPC; they need no knowledge of P2P networking.
5. **Adaptive resource utilization**: The swarm dynamically adjusts decomposition depth to fully utilize all available agents.

---

## 3. Base Protocol

### 3.1 Message Format

All WWS messages use JSON-RPC 2.0 envelope format with an additional `signature` field.

#### 3.1.1 Request

```json
{
  "jsonrpc": "2.0",
  "method": "<namespace>.<action>",
  "id": "<uuid-v4>",
  "params": { ... },
  "signature": "<hex-encoded Ed25519 signature>"
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `jsonrpc` | string | REQUIRED | MUST be `"2.0"`. |
| `method` | string | REQUIRED | Protocol method in `namespace.action` format. |
| `id` | string | OPTIONAL | Request ID. MUST be present for requests expecting a response. Omitted for notifications. |
| `params` | object | REQUIRED | Method-specific parameters. |
| `signature` | string | REQUIRED | Hex-encoded Ed25519 signature over `canonical_json({"method": method, "params": params})`. |

**Signature computation:**
1. Construct the canonical signing payload: `{"method": "<method>", "params": <params>}`
2. Serialize to JSON with keys sorted alphabetically (canonical JSON).
3. Sign the UTF-8 bytes with the sender's Ed25519 private key.
4. Hex-encode the 64-byte signature.

#### 3.1.2 Response

```json
{
  "jsonrpc": "2.0",
  "id": "<matching-request-id>",
  "result": { ... }
}
```

Error response:

```json
{
  "jsonrpc": "2.0",
  "id": "<matching-request-id>",
  "error": {
    "code": -32600,
    "message": "Invalid Request",
    "data": { ... }
  }
}
```

#### 3.1.3 Notification

A notification is a request without an `id` field. The sender does not expect a response.
Used for: `swarm.keepalive`, `election.candidacy`, `consensus.proposal_commit`.

#### 3.1.4 Batching

Implementations MAY support JSON-RPC batch requests (array of request objects).
If supported, responses MUST be returned as a corresponding array.

### 3.2 Transport Layer

#### 3.2.1 P2P Transport (Inter-Node)

Inter-node communication uses libp2p with the following configuration:

| Component | Choice | Rationale |
|-----------|--------|-----------|
| **Transport** | TCP + QUIC | TCP for reliability; QUIC for NAT traversal and multiplexing. |
| **Security** | Noise XX | Authenticated encryption with Ed25519 identity keys. |
| **Multiplexing** | Yamux | Stream multiplexing over a single connection. |
| **Peer Discovery** | Kademlia DHT + mDNS | DHT for global discovery; mDNS for zero-conf local discovery. |
| **Pub/Sub** | GossipSub v1.1 | Topic-based message dissemination with mesh peering. |
| **NAT Traversal** | AutoNAT + Relay | Detect NAT status; use relay nodes for unreachable peers. |
| **Peer Identity** | Ed25519 | Same keypair used for both libp2p identity and message signing. |

#### 3.2.2 Local Transport (Connector ↔ Agent)

The WWS.Connector exposes a JSON-RPC 2.0 server to the local Agent over:
- **TCP** on `127.0.0.1:<port>` (default port: `9390`)
- **Unix Domain Socket** at `/tmp/openswarm-connector.sock` (preferred on Unix systems)

Messages on the local transport do NOT require the `signature` field.

#### 3.2.3 GossipSub Topics

All GossipSub topics use the prefix `/openswarm/1.0.0/`.

| Topic | Pattern | Purpose |
|-------|---------|---------|
| Election (Tier-1) | `/openswarm/1.0.0/election/tier1` | Candidacy announcements and election votes. |
| Proposals | `/openswarm/1.0.0/proposals/<task_id>` | Proposal commits and reveals for a specific task. |
| Voting | `/openswarm/1.0.0/voting/<task_id>` | Ranked choice votes for a specific task. |
| Tasks (per tier) | `/openswarm/1.0.0/tasks/tier<N>` | Task assignments for agents at tier N. |
| Results | `/openswarm/1.0.0/results/<task_id>` | Result submissions for a specific task. |
| Keep-alive | `/openswarm/1.0.0/keepalive` | Periodic liveness signals. |
| Hierarchy | `/openswarm/1.0.0/hierarchy` | Tier assignments and succession announcements. |

### 3.3 Identity

Each node's identity is derived from an Ed25519 keypair:

1. Generate an Ed25519 signing key (32 bytes random seed).
2. Derive the verifying (public) key.
3. Compute `NodeID = SHA-256(public_key_bytes)`.
4. The agent's DID is: `did:swarm:<hex(NodeID)>`.

The same keypair serves as:
- The libp2p `PeerId` (via Ed25519 identity)
- The Kademlia node ID
- The message signing key

---

## 4. Connection Lifecycle

### 4.1 State Machine

```
                    ┌──────────┐
                    │          │
            ┌──────►Discovered│
            │       │          │
            │       └────┬─────┘
            │            │ handshake
            │            ▼
            │       ┌──────────┐
            │       │          │
            │       │Connecting│
            │       │          │
            │       └────┬─────┘
            │            │ handshake_ack + PoW verified
            │            ▼
            │       ┌──────────┐
     timeout│       │          │
            │       │Connected │
            │       │          │
            │       └────┬─────┘
            │            │ epoch sync + tier assignment
            │            ▼
            │       ┌──────────┐
            │       │          │
            │       │  Active  │◄──── normal operation
            │       │          │
            │       └────┬─────┘
            │            │
            │       ┌────┴─────┐
            │       │          │
            └───────┤Disconnect│
                    │          │
                    └──────────┘
```

### 4.2 Bootstrap Process

When a WWS.Connector starts:

1. **Key Generation**: If no existing keypair is found, generate a new Ed25519 keypair and persist it.
2. **Local Discovery (mDNS)**: Broadcast an mDNS query on the local network. If peers respond, establish local connections immediately.
3. **Global Discovery (Bootstrap)**: Connect to hardcoded Bootstrap Nodes. These nodes provide a list of active peers but do NOT control the swarm.
4. **DHT Integration**: The node takes its place in the Kademlia ring based on its `NodeID`, populating k-buckets with nearby peers.
5. **Handshake**: Exchange `swarm.handshake` messages with discovered peers, including Proof of Work.
6. **Epoch Sync**: Obtain current epoch number, tier assignments, and active task state from connected peers via CRDT sync.
7. **Role Assignment**: Participate in the next hierarchy formation cycle (or join an existing tier if mid-epoch).

### 4.3 Handshake Message

**Method:** `swarm.handshake`
**Direction:** Bidirectional (both peers send)
**Response expected:** Yes

**Request:**

```json
{
  "jsonrpc": "2.0",
  "method": "swarm.handshake",
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "params": {
    "agent_id": "did:swarm:a1b2c3d4...",
    "pub_key": "MCowBQYDK2Vw...",
    "capabilities": ["gpt-4", "python-exec", "web-search"],
    "resources": {
      "cpu_cores": 8,
      "ram_gb": 32,
      "gpu_vram_gb": null,
      "disk_gb": 100
    },
    "location_vector": { "x": 0.45, "y": 0.12, "z": 0.99 },
    "proof_of_work": {
      "nonce": 283741,
      "hash": "0000a3f2...",
      "difficulty": 16
    },
    "protocol_version": "/openswarm/1.0.0"
  },
  "signature": "3045..."
}
```

**Response (Success):**

```json
{
  "jsonrpc": "2.0",
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "result": {
    "accepted": true,
    "agent_id": "did:swarm:e5f6g7h8...",
    "current_epoch": 105,
    "estimated_swarm_size": 850,
    "hierarchy_depth": 3,
    "your_tier": null
  }
}
```

The receiver MUST:
1. Verify the Ed25519 signature.
2. Verify the Proof of Work hash meets the required difficulty.
3. Verify the `protocol_version` is compatible.
4. Store the peer's profile for reputation tracking.

### 4.4 Keep-Alive

**Method:** `swarm.keepalive`
**Direction:** Notification (no response)
**Interval:** Every 10 seconds

```json
{
  "jsonrpc": "2.0",
  "method": "swarm.keepalive",
  "params": {
    "agent_id": "did:swarm:a1b2c3d4...",
    "epoch": 105,
    "timestamp": "2026-02-07T12:00:00Z"
  },
  "signature": "3045..."
}
```

If a Tier-1 leader's keep-alive is not received for **30 seconds** (3 missed intervals),
the succession protocol (§5.6) MUST be triggered.

---

## 5. Dynamic Hierarchy

### 5.1 Overview

The swarm organizes into a **Dynamic Pyramid** with branching factor `k` (default: 10).
The hierarchy depth is computed dynamically based on the estimated swarm size `N`.

### 5.2 Hierarchy Depth Calculation

```
depth = ceil(log_k(N))
```

| Swarm Size (N) | k=10 | Tiers |
|----------------|------|-------|
| 1–10 | 1 | Tier-1 only (all are orchestrators) |
| 11–100 | 2 | Tier-1 (10) + Tier-2 (up to 90) |
| 101–1,000 | 3 | Tier-1 (10) + Tier-2 (up to 100) + Tier-3 (up to 890) |
| 1,001–10,000 | 4 | Four tiers |
| 10,001–100,000 | 5 | Five tiers |

The last tier MAY be partial (not all slots filled).

### 5.3 Swarm Size Estimation

Each node estimates `N` using Kademlia bucket density:

1. For each k-bucket `i` (distance `2^i` to `2^(i+1)`), count the known peers `c_i`.
2. The expected density at distance `d` is `N / 2^256`.
3. Estimate: `N ≈ 2^256 × (Σ c_i) / (Σ bucket_range_i)`.

In practice, a simpler heuristic is used:
- Count total known peers in routing table: `P`.
- Use the correction factor: `N ≈ P × (2^(b × k_bucket_count) / filled_buckets)`.

Implementations SHOULD use exponential moving average to smooth the estimate.

### 5.4 Tier-1 Elections (Global Leaders)

Elections occur at the start of each epoch.

**Phase 1: Candidacy Announcement**

Agents with `composite_score > threshold` broadcast a candidacy:

**Method:** `election.candidacy`
**Direction:** Notification (GossipSub topic: `election/tier1`)

```json
{
  "jsonrpc": "2.0",
  "method": "election.candidacy",
  "params": {
    "agent_id": "did:swarm:a1b2c3d4...",
    "epoch": 106,
    "score": {
      "agent_id": "did:swarm:a1b2c3d4...",
      "proof_of_compute": 0.85,
      "reputation": 0.92,
      "uptime": 0.99,
      "stake": 0.5
    },
    "location_vector": { "x": 0.45, "y": 0.12, "z": 0.99 }
  },
  "signature": "3045..."
}
```

**Phase 2: Voting**

All agents vote for candidates using Ranked Choice:

**Method:** `election.vote`
**Direction:** Notification (GossipSub topic: `election/tier1`)

```json
{
  "jsonrpc": "2.0",
  "method": "election.vote",
  "params": {
    "voter": "did:swarm:x9y8z7...",
    "epoch": 106,
    "candidate_rankings": [
      "did:swarm:a1b2c3d4...",
      "did:swarm:e5f6g7h8...",
      "did:swarm:i9j0k1l2..."
    ]
  },
  "signature": "3045..."
}
```

**Phase 3: Tallying**

Votes are counted using Instant Runoff Voting (IRV):
1. Count first-choice votes for each candidate.
2. If a candidate has > `N/k` first-choice votes, they win a Tier-1 seat.
3. If no candidate reaches the threshold, eliminate the candidate with fewest first-choice votes.
4. Redistribute eliminated candidate's votes to each voter's next choice.
5. Repeat until `k` seats are filled.

### 5.5 Composite Score

The composite score determines election eligibility:

```
S = 0.25 × PoC + 0.40 × Reputation + 0.20 × Uptime + 0.15 × Stake
```

| Component | Range | Description |
|-----------|-------|-------------|
| **Proof of Compute (PoC)** | 0.0–1.0 | Computational benchmark score at connection time. |
| **Reputation** | 0.0–1.0 | Historical task success rate, stored in distributed ledger. |
| **Uptime** | 0.0–1.0 | Fraction of current epoch the node has been online. |
| **Stake** | 0.0–1.0 | Optional anti-Sybil stake (normalized). |

### 5.6 Geo-Clustering (Tier-2+ Assignment)

After Tier-1 leaders are elected:

1. Each Tier-1 leader announces `k` vacancies for Tier-2.
2. Each remaining agent measures latency (or computes Vivaldi distance) to all `k` Tier-1 leaders.
3. The agent joins the Tier-1 leader with the **lowest latency**.
4. Within each branch, the top-scoring agents become Tier-2 coordinators; the rest become Tier-3 (or lower).
5. The process repeats recursively until all agents have tier assignments.

**Method:** `hierarchy.assign_tier`
**Direction:** Parent → Subordinate

```json
{
  "jsonrpc": "2.0",
  "method": "hierarchy.assign_tier",
  "id": "assign-001",
  "params": {
    "assigned_agent": "did:swarm:x9y8z7...",
    "tier": { "TierN": 2 },
    "parent_id": "did:swarm:a1b2c3d4...",
    "epoch": 106,
    "branch_size": 85
  },
  "signature": "3045..."
}
```

### 5.7 Succession Protocol

If a Tier-1 leader becomes unresponsive (no keep-alive for 30 seconds):

1. Tier-2 subordinates detect the timeout.
2. The Tier-2 agent with the highest composite score initiates succession.
3. A `hierarchy.succession` message is broadcast to the branch.
4. The successor assumes the leader role, restoring state from CRDT replicas.
5. The swarm continues without reset.

**Method:** `hierarchy.succession`
**Direction:** Notification (GossipSub topic: `hierarchy`)

```json
{
  "jsonrpc": "2.0",
  "method": "hierarchy.succession",
  "params": {
    "failed_leader": "did:swarm:a1b2c3d4...",
    "new_leader": "did:swarm:x9y8z7...",
    "epoch": 106,
    "branch_agents": ["did:swarm:m1n2...", "did:swarm:o3p4..."]
  },
  "signature": "3045..."
}
```

---

## 6. Competitive Planning & Consensus

### 6.1 Overview

When a task enters the swarm, it undergoes a three-phase process:
1. **RFP (Request for Proposal)**: Tier-1 agents independently propose decomposition plans.
2. **Voting (RCV)**: Agents vote on proposals using Ranked Choice Voting.
3. **Cascade**: The winning plan's subtasks are distributed down the hierarchy, and the process repeats recursively.

### 6.2 Task Injection

External tasks enter through any Tier-1 agent:

**Method:** `task.inject`
**Direction:** External → Tier-1

```json
{
  "jsonrpc": "2.0",
  "method": "task.inject",
  "id": "inject-001",
  "params": {
    "task": {
      "task_id": "task-550e8400...",
      "parent_task_id": null,
      "epoch": 106,
      "status": "Pending",
      "description": "Build a comprehensive market analysis report for Q1 2026",
      "assigned_to": null,
      "tier_level": 1,
      "subtasks": [],
      "created_at": "2026-02-07T12:00:00Z",
      "deadline": "2026-02-08T12:00:00Z"
    },
    "originator": "did:swarm:external..."
  },
  "signature": "3045..."
}
```

### 6.3 Proposal Phase (Commit-Reveal)

To prevent plagiarism, proposals use a two-phase commit-reveal scheme:

**Phase 1 — Commit (hash only):**

**Method:** `consensus.proposal_commit`
**Direction:** Notification (GossipSub topic: `proposals/<task_id>`)

```json
{
  "jsonrpc": "2.0",
  "method": "consensus.proposal_commit",
  "params": {
    "task_id": "task-550e8400...",
    "proposer": "did:swarm:a1b2c3d4...",
    "epoch": 106,
    "plan_hash": "e3b0c44298fc1c149..."
  },
  "signature": "3045..."
}
```

Wait condition: All `k` Tier-1 agents MUST submit commits, OR a timeout of 60 seconds
elapses (whichever comes first).

**Phase 2 — Reveal (full plan):**

**Method:** `consensus.proposal_reveal`
**Direction:** Notification (GossipSub topic: `proposals/<task_id>`)

```json
{
  "jsonrpc": "2.0",
  "method": "consensus.proposal_reveal",
  "params": {
    "task_id": "task-550e8400...",
    "plan": {
      "plan_id": "plan-a1b2c3d4...",
      "task_id": "task-550e8400...",
      "proposer": "did:swarm:a1b2c3d4...",
      "epoch": 106,
      "subtasks": [
        {
          "index": 0,
          "description": "Gather macroeconomic indicators",
          "required_capabilities": ["web-search", "data-analysis"],
          "estimated_complexity": 0.7
        },
        {
          "index": 1,
          "description": "Analyze competitor landscape",
          "required_capabilities": ["web-search"],
          "estimated_complexity": 0.6
        }
      ],
      "rationale": "Decompose into 10 parallel research streams...",
      "estimated_parallelism": 0.85,
      "created_at": "2026-02-07T12:01:00Z"
    }
  },
  "signature": "3045..."
}
```

The receiver MUST verify: `SHA-256(canonical_json(plan)) == plan_hash` from the commit phase.

### 6.4 Voting Phase (Ranked Choice Voting)

**Electorate:**
- All `k` Tier-1 agents (each ranks all plans EXCEPT their own — self-vote prohibition).
- A random sample of Tier-2 agents ("Senate") — size: `min(k, tier2_count / 2)`.

**Method:** `consensus.vote`
**Direction:** Notification (GossipSub topic: `voting/<task_id>`)

```json
{
  "jsonrpc": "2.0",
  "method": "consensus.vote",
  "params": {
    "task_id": "task-550e8400...",
    "epoch": 106,
    "voter": "did:swarm:x9y8z7...",
    "rankings": ["plan-a1b2c3d4...", "plan-e5f6g7h8...", "plan-i9j0k1l2..."],
    "critic_scores": {
      "plan-a1b2c3d4...": {
        "feasibility": 0.9,
        "parallelism": 0.8,
        "completeness": 0.85,
        "risk": 0.2
      },
      "plan-e5f6g7h8...": {
        "feasibility": 0.7,
        "parallelism": 0.6,
        "completeness": 0.9,
        "risk": 0.4
      }
    }
  },
  "signature": "3045..."
}
```

**IRV Counting Algorithm:**

1. Count first-choice votes for each plan.
2. If any plan has > 50% of first-choice votes → that plan wins.
3. Otherwise, eliminate the plan with the fewest first-choice votes.
4. Redistribute eliminated plan's ballots to each voter's next-ranked choice.
5. Repeat steps 2–4 until a plan achieves > 50%.

**Tie-breaking:** If two plans tie, the plan with the higher aggregate `critic_score` wins.

### 6.5 Recursive Decomposition Cascade

After the winning plan is selected:

1. The proposer of the winning plan becomes the **Prime Orchestrator** for this task.
2. The `k` subtasks from the winning plan are assigned to the `k` Tier-1 agents.
3. Each Tier-1 agent now has a subtask. It initiates a NEW RFP cycle with its `k` Tier-2 subordinates.
4. This process repeats recursively down the hierarchy until a **stop condition** is met.

**Stop conditions:**
- The task is atomic (cannot be meaningfully decomposed).
- The current tier is the bottom of the hierarchy (Executor level).
- The subtask complexity is below a configurable threshold.

**Method:** `task.assign`
**Direction:** Parent → Subordinate

```json
{
  "jsonrpc": "2.0",
  "method": "task.assign",
  "id": "assign-task-001",
  "params": {
    "task": {
      "task_id": "task-sub-001...",
      "parent_task_id": "task-550e8400...",
      "epoch": 106,
      "status": "InProgress",
      "description": "Gather macroeconomic indicators for US market",
      "assigned_to": "did:swarm:m1n2...",
      "tier_level": 2,
      "subtasks": [],
      "created_at": "2026-02-07T12:05:00Z",
      "deadline": "2026-02-07T18:00:00Z"
    },
    "assignee": "did:swarm:m1n2...",
    "parent_task_id": "task-550e8400...",
    "winning_plan_id": "plan-a1b2c3d4..."
  },
  "signature": "3045..."
}
```

### 6.6 Message Flow Diagram

```
  External       Tier-1 (×k)       Tier-2 (×k²)     Tier-N (Executors)
     │                │                  │                    │
     │──task.inject──►│                  │                    │
     │                │                  │                    │
     │         ┌──────┴──────┐           │                    │
     │         │ Each agent  │           │                    │
     │         │ generates a │           │                    │
     │         │ plan (LLM)  │           │                    │
     │         └──────┬──────┘           │                    │
     │                │                  │                    │
     │         proposal_commit ────────► │ (observed by       │
     │         (all k agents)            │  Senate sample)    │
     │                │                  │                    │
     │         proposal_reveal ────────► │                    │
     │                │                  │                    │
     │         consensus.vote ◄────────► │                    │
     │         (mutual + Senate)         │                    │
     │                │                  │                    │
     │         ┌──────┴──────┐           │                    │
     │         │ IRV tally:  │           │                    │
     │         │ Plan wins   │           │                    │
     │         └──────┬──────┘           │                    │
     │                │                  │                    │
     │         task.assign ────────────► │                    │
     │                │                  │                    │
     │                │           ┌──────┴──────┐             │
     │                │           │ Recursive   │             │
     │                │           │ RFP at each │             │
     │                │           │ tier...      │             │
     │                │           └──────┬──────┘             │
     │                │                  │                    │
     │                │                  │──task.assign──────►│
     │                │                  │                    │
     │                │                  │                    │── execute
     │                │                  │                    │
     │                │                  │◄──submit_result────│
     │                │                  │                    │
     │                │◄──submit_result──│                    │
     │                │                  │                    │
     │◄──final_result─│                  │                    │
```

---

## 7. Task Execution & Verification

### 7.1 Execution

When an Executor (leaf node) receives an atomic task:

1. The WWS.Connector converts the task into the agent's native format (e.g., OpenClaw prompt).
2. The agent executes the task and produces an output.
3. The Connector packages the output as an Artifact with a Content ID (CID).

### 7.2 Result Submission

**Method:** `task.submit_result`
**Direction:** Subordinate → Parent

```json
{
  "jsonrpc": "2.0",
  "method": "task.submit_result",
  "id": "result-001",
  "params": {
    "task_id": "task-sub-001...",
    "agent_id": "did:swarm:exec1...",
    "artifact": {
      "artifact_id": "art-001...",
      "task_id": "task-sub-001...",
      "producer": "did:swarm:exec1...",
      "content_cid": "QmYwAPJzv5CZsnA...",
      "merkle_hash": "a3f2b1c4d5e6...",
      "content_type": "application/json",
      "size_bytes": 4096,
      "created_at": "2026-02-07T14:00:00Z"
    },
    "merkle_proof": ["hash1...", "hash2...", "hash3..."]
  },
  "signature": "3045..."
}
```

### 7.3 Merkle-DAG Verification

Results are verified bottom-up using a Merkle-DAG structure:

1. **Leaf hash**: Each Executor computes `H_leaf = SHA-256(artifact_content)`.
2. **Branch hash**: Each Coordinator collects hashes from its `k` subordinates and computes:
   ```
   H_branch = SHA-256(H_child_0 || H_child_1 || ... || H_child_{k-1})
   ```
   Children are ordered by their task index (0 to k-1).
3. **Root hash**: Tier-1 computes the root hash from its branch hashes.
4. **Verification**: Any node can verify a result by recomputing hashes from the proof chain.

### 7.4 Coordinator Review

When a Coordinator receives results from subordinates:

1. **Automated check**: Run an LLM-Validator (Judge) to verify result compliance with the task.
2. **Accept**: If valid, merge results into a summary report and compute the branch Merkle hash.
3. **Reject**: If invalid, return the task for rework OR reassign to a backup agent.

**Method:** `task.verification`
**Direction:** Parent → Subordinate

```json
{
  "jsonrpc": "2.0",
  "method": "task.verification",
  "id": "verify-001",
  "params": {
    "task_id": "task-sub-001...",
    "agent_id": "did:swarm:exec1...",
    "accepted": true,
    "reason": null
  },
  "signature": "3045..."
}
```

### 7.5 Final Assembly

The Prime Orchestrator:
1. Collects verified results from all Tier-1 agents.
2. Compiles the final response.
3. Computes the swarm Merkle root hash.
4. Signs the final result with the swarm's collective verification.
5. Returns the result to the original task injector.

---

## 8. State Management

### 8.1 Hot State: CRDT (Conflict-free Replicated Data Types)

For synchronizing real-time state across the swarm, WWS uses **Observed-Remove Sets (OR-Sets)**.

#### 8.1.1 Managed State

| State | CRDT Type | Description |
|-------|-----------|-------------|
| Task Registry | OR-Set of `(task_id, TaskStatus)` | Current status of all active tasks. |
| Agent Registry | OR-Set of `(agent_id, Tier, parent_id)` | Current tier assignments. |
| Epoch State | LWW-Register | Current epoch number and metadata. |

#### 8.1.2 Merge Semantics

OR-Sets support concurrent add/remove operations:
- **Add**: Generates a unique tag for each element addition.
- **Remove**: Removes all known tags for an element.
- **Merge**: Union of all adds minus all removes.

This ensures mathematically correct merging without conflicts or locks, even with connection drops.

#### 8.1.3 Synchronization

CRDT state is synchronized via:
1. **Piggybacking**: State deltas are attached to keep-alive messages.
2. **Anti-entropy**: Periodic full-state exchange between neighbors (every 60 seconds).
3. **On-demand**: Explicit state request during handshake or succession.

### 8.2 Cold Context: Content-Addressed Storage

Large data (task descriptions, artifacts, logs) is stored using content-addressing:

1. The producing agent stores data locally.
2. Computes `CID = SHA-256(data)`.
3. Publishes a provider record to the Kademlia DHT: `CID → agent_id`.
4. Consumers retrieve data by:
   a. Looking up providers for the CID in the DHT.
   b. Requesting the data directly from the provider via peer-to-peer streaming.

### 8.3 Data Retention

- **Hot state**: Retained for `current_epoch + 2` (sliding window).
- **Cold context**: Retained for the duration of the task + configurable grace period.
- **Artifacts**: Pinned by the task originator; unpinned data MAY be garbage-collected.

---

## 9. Adaptive Granularity

### 9.1 Overview

Every planning agent at level `L` knows:
- `N` — total agents in the swarm (from DHT estimation).
- `N_branch` — agents in its specific command branch (`N / k^L`).

### 9.2 Utilization Formula

When decomposing a task, the agent creates `S` subtasks such that:

```
S ≈ min(k, N_branch / k)
```

The goal is to ensure every agent in the branch has work.

### 9.3 Decomposition Strategies

| Condition | Strategy | Description |
|-----------|----------|-------------|
| `N_branch > k²` | **Massive Parallelism** | Create `k` subtasks, each designed for further decomposition. Force deep recursion. |
| `k < N_branch ≤ k²` | **Standard Decomposition** | Create `k` subtasks, each assigned to a subordinate. |
| `N_branch ≤ k` | **Direct Assignment** | Assign the task directly without further decomposition. |
| Task is atomic, `N_branch > 1` | **Redundant Execution** | Assign the same task to multiple sub-branches for reliability or variation. |

### 9.4 Redundant Execution

When a task cannot be decomposed but idle agents exist:
- The same task is assigned to `min(N_branch, k)` agents.
- Results are compared for consensus (majority agreement = accepted).
- OR: Results are collected as variations for human selection.

---

## 10. Agent Integration (WWS.Connector)

### 10.1 Overview

The WWS.Connector is a sidecar process providing agents with a simple local API.
The agent needs no knowledge of P2P networking, consensus, or hierarchy — it simply
receives tasks and returns results.

### 10.2 Swarm API (Local JSON-RPC)

The following methods are available on the local JSON-RPC server:

#### `swarm.connect`

Initialize connection to the swarm.

**Request:**
```json
{
  "jsonrpc": "2.0",
  "method": "swarm.connect",
  "id": "1",
  "params": {
    "capabilities": ["gpt-4", "python-exec"],
    "resources": { "cpu_cores": 8, "ram_gb": 32 }
  }
}
```

**Response:**
```json
{
  "jsonrpc": "2.0",
  "id": "1",
  "result": {
    "agent_id": "did:swarm:a1b2c3d4...",
    "connected": true,
    "swarm_size": 850,
    "epoch": 106
  }
}
```

#### `swarm.get_network_stats`

Returns network statistics including swarm size and hierarchy info.

**Request:**
```json
{
  "jsonrpc": "2.0",
  "method": "swarm.get_network_stats",
  "id": "2",
  "params": {}
}
```

**Response:**
```json
{
  "jsonrpc": "2.0",
  "id": "2",
  "result": {
    "total_agents": 850,
    "hierarchy_depth": 3,
    "branching_factor": 10,
    "current_epoch": 106,
    "my_tier": { "TierN": 2 },
    "subordinate_count": 8,
    "parent_id": "did:swarm:a1b2c3d4..."
  }
}
```

#### `swarm.propose_plan`

Submit a decomposition plan for voting.

**Request:**
```json
{
  "jsonrpc": "2.0",
  "method": "swarm.propose_plan",
  "id": "3",
  "params": {
    "task_id": "task-550e8400...",
    "plan": { ... }
  }
}
```

#### `swarm.submit_result`

Submit a completed task result.

**Request:**
```json
{
  "jsonrpc": "2.0",
  "method": "swarm.submit_result",
  "id": "4",
  "params": {
    "task_id": "task-sub-001...",
    "content": "...",
    "content_type": "application/json"
  }
}
```

#### `swarm.receive_task`

Long-poll or subscribe to incoming task assignments.

**Request:**
```json
{
  "jsonrpc": "2.0",
  "method": "swarm.receive_task",
  "id": "5",
  "params": {
    "timeout_ms": 30000
  }
}
```

#### `swarm.get_status`

Get current agent status within the swarm.

**Request:**
```json
{
  "jsonrpc": "2.0",
  "method": "swarm.get_status",
  "id": "6",
  "params": {}
}
```

### 10.3 MCP Compatibility

The WWS.Connector MAY expose an MCP Server interface, allowing agents to
use the swarm as a Tool:

```json
{
  "name": "use_swarm",
  "description": "Delegate a complex task to the AI swarm for collective execution",
  "inputSchema": {
    "type": "object",
    "properties": {
      "task": { "type": "string", "description": "Task description" },
      "deadline_minutes": { "type": "integer" }
    },
    "required": ["task"]
  }
}
```

### 10.4 Agent Bridge

The Connector translates between protocol messages and agent-native formats:

| Direction | Translation |
|-----------|------------|
| **Inbound** | `task.assign` JSON → Agent prompt/command format |
| **Outbound** | Agent output → `task.submit_result` with CID and Merkle hash |

---

## 11. Error Handling

### 11.1 Error Categories

| Category | Code Range | Description |
|----------|------------|-------------|
| JSON-RPC Standard | -32700 to -32600 | Parse errors, invalid requests, method not found. |
| Protocol Errors | -32000 to -32099 | Signature failures, epoch mismatches, PoW invalid. |
| Consensus Errors | -31000 to -31099 | Self-vote, duplicate proposal, timeout. |
| Task Errors | -30000 to -30099 | Task not found, result rejected, deadline exceeded. |
| Network Errors | -29000 to -29099 | Peer unreachable, DHT lookup failed. |

### 11.2 Standard Error Codes

| Code | Message | Description |
|------|---------|-------------|
| -32700 | Parse error | Invalid JSON. |
| -32600 | Invalid Request | Missing required fields. |
| -32601 | Method not found | Unknown method name. |
| -32602 | Invalid params | Malformed parameters. |
| -32000 | Invalid signature | Ed25519 signature verification failed. |
| -32001 | Epoch mismatch | Message epoch does not match current epoch. |
| -32002 | Invalid PoW | Proof of Work does not meet difficulty. |
| -32003 | Insufficient reputation | Agent score below threshold. |
| -31000 | Self-vote prohibited | Agent attempted to vote for own proposal. |
| -31001 | Duplicate proposal | Agent already submitted a proposal for this task. |
| -31002 | Commit-reveal mismatch | Revealed plan hash does not match commit. |
| -31003 | Voting timeout | Voting phase exceeded time limit. |
| -30000 | Task not found | Referenced task ID does not exist. |
| -30001 | Result rejected | Artifact failed validation. |
| -30002 | Deadline exceeded | Task was not completed within deadline. |

### 11.3 Error Recovery

| Failure | Recovery |
|---------|----------|
| Agent disconnects mid-task | Task reassigned to backup agent. Coordinator retains state via CRDT. |
| Tier-1 leader fails | Succession protocol (§5.7). Tier-2 takes over. |
| Voting fails (timeout) | Extend timeout by 2x. If fails again, highest critic-score plan wins by default. |
| Result validation fails | Task returned to executor with rejection reason. Max 3 retries before reassignment. |
| Network partition | Each partition continues independently. CRDT merge reconciles on reconnection. |

---

## 12. Security

### 12.1 Threat Model

| Threat | Mitigation |
|--------|-----------|
| **Sybil Attack** (mass fake agents) | Proof of Work entry cost + stake requirement. |
| **Message Forgery** | Ed25519 signatures on all messages. |
| **Plagiarism** (copying plans) | Commit-reveal scheme for proposals. |
| **Collusion** (coordinated voting) | Senate sampling (Tier-2 voters) + ranked choice voting. |
| **Man-in-the-Middle** | Noise XX authenticated encryption on all connections. |
| **Data Tampering** | Merkle-DAG verification on all results. |
| **Leader Capture** | Epoch-based re-election + reputation decay. |
| **Eclipse Attack** | Diverse routing table maintenance + multiple bootstrap nodes. |

### 12.2 Proof of Work

Upon connection, each agent MUST solve a PoW puzzle:
- Difficulty: 16 leading zero bits (adjustable per swarm configuration).
- Input: `agent_id || timestamp || nonce`.
- The resulting hash MUST have the required number of leading zero bits.

### 12.3 Web of Trust

Agents MAY carry endorsements from trusted Bootstrap nodes:
- Endorsement: Ed25519 signature from a bootstrap node over the agent's public key.
- Endorsed agents receive a reputation bonus during elections.
- Long DHT history (> 1 epoch) provides implicit trust.

### 12.4 Transport Encryption

All peer-to-peer connections MUST use Noise XX protocol:
- Provides mutual authentication via Ed25519 keys.
- Provides forward secrecy.
- Encrypts all traffic on the wire.

---

## 13. Schema Reference

### 13.1 Core Data Types

#### AgentId
```
type AgentId = string  // Format: "did:swarm:<64-char-hex>"
```

#### VivaldiCoordinates
```json
{
  "x": 0.0,
  "y": 0.0,
  "z": 0.0
}
```

#### AgentResources
```json
{
  "cpu_cores": 8,
  "ram_gb": 32,
  "gpu_vram_gb": null,
  "disk_gb": 100
}
```

#### NodeScore
```json
{
  "agent_id": "did:swarm:...",
  "proof_of_compute": 0.85,
  "reputation": 0.92,
  "uptime": 0.99,
  "stake": 0.5
}
```

#### TaskStatus
```
enum TaskStatus = "Pending" | "ProposalPhase" | "VotingPhase" | "InProgress"
                | "Completed" | "Failed" | "Rejected"
```

#### Tier
```
enum Tier = "Tier1" | "Tier2" | { "TierN": <uint32> } | "Executor"
```

#### CriticScore
```json
{
  "feasibility": 0.9,
  "parallelism": 0.8,
  "completeness": 0.85,
  "risk": 0.2
}
```

#### ProofOfWork
```json
{
  "nonce": 283741,
  "hash": "0000a3f2...",
  "difficulty": 16
}
```

### 13.2 Method Registry

| Method | Direction | Response | GossipSub Topic |
|--------|-----------|----------|-----------------|
| `swarm.handshake` | Bidirectional | Yes | Direct (not pub/sub) |
| `swarm.keepalive` | Notification | No | `keepalive` |
| `election.candidacy` | Notification | No | `election/tier1` |
| `election.vote` | Notification | No | `election/tier1` |
| `hierarchy.assign_tier` | Parent→Child | Yes | Direct |
| `hierarchy.succession` | Notification | No | `hierarchy` |
| `task.inject` | External→Tier-1 | Yes | Direct |
| `consensus.proposal_commit` | Notification | No | `proposals/<task_id>` |
| `consensus.proposal_reveal` | Notification | No | `proposals/<task_id>` |
| `consensus.vote` | Notification | No | `voting/<task_id>` |
| `task.assign` | Parent→Child | Yes | Direct |
| `task.submit_result` | Child→Parent | Yes | `results/<task_id>` |
| `task.verification` | Parent→Child | No | Direct |

### 13.3 Local API Methods (Connector ↔ Agent)

| Method | Description |
|--------|-------------|
| `swarm.connect` | Initialize swarm connection with agent capabilities. |
| `swarm.get_network_stats` | Get swarm size, hierarchy depth, tier assignment. |
| `swarm.propose_plan` | Submit a task decomposition plan for voting. |
| `swarm.submit_result` | Submit a completed task artifact. |
| `swarm.receive_task` | Long-poll for incoming task assignments. |
| `swarm.get_status` | Get current agent status in the swarm. |

---

## 14. Appendices

### 14.1 Configuration Parameters

| Parameter | Default | Description |
|-----------|---------|-------------|
| `branching_factor` | 10 | Number of subordinates per coordinator (k). |
| `epoch_duration_secs` | 3600 | Epoch duration in seconds. |
| `keepalive_interval_secs` | 10 | Keep-alive ping interval. |
| `leader_timeout_secs` | 30 | Leader failover timeout. |
| `commit_reveal_timeout_secs` | 60 | Proposal commit-reveal phase timeout. |
| `voting_timeout_secs` | 120 | Voting phase timeout. |
| `pow_difficulty` | 16 | Proof of Work difficulty (leading zero bits). |
| `max_hierarchy_depth` | 10 | Maximum pyramid depth. |
| `rpc_port` | 9390 | Local JSON-RPC server port. |
| `bootstrap_nodes` | [] | List of bootstrap node multiaddrs. |

### 14.2 References

- [RFC 2119](https://www.rfc-editor.org/rfc/rfc2119) — Key words for use in RFCs
- [JSON-RPC 2.0](https://www.jsonrpc.org/specification) — JSON-RPC Specification
- [libp2p](https://docs.libp2p.io/) — Modular peer-to-peer networking stack
- [Kademlia](https://pdos.csail.mit.edu/~petar/papers/maymounkov-kademlia-lncs.pdf) — DHT protocol
- [GossipSub v1.1](https://github.com/libp2p/specs/blob/master/pubsub/gossipsub/gossipsub-v1.1.md) — Pub/Sub protocol
- [Ed25519](https://ed25519.cr.yp.to/) — Digital signature scheme
- [CRDT](https://crdt.tech/) — Conflict-free Replicated Data Types
- [Vivaldi](https://www.cs.cornell.edu/projects/Vivaldi/) — Network coordinate system
- [MCP](https://spec.modelcontextprotocol.io/) — Model Context Protocol

### 14.3 Changelog

| Date | Version | Changes |
|------|---------|---------|
| 2026-02-07 | 0.1.0 | Initial draft specification. |
