# Web Dashboard

The `wws-connector` serves a live web dashboard at `http://127.0.0.1:9371/` (or whichever HTTP port you configured). Open it in any browser while the connector is running.

---

## Panels

### Cosmic Canvas (top-left)

Animated graph of all agents in the swarm. Nodes are colored by **status** (not tier):

- **Blue-white (hue 220)** — Healthy / active
- **Amber (hue 42)** — Degraded / slow heartbeat
- **Coral-red (hue 0)** — Offline / unreachable

Connections between nodes are drawn from the `/api/topology` edges (peer links), with alpha fading based on canvas-diagonal distance. Nodes use an elliptical layout that fills widescreen viewports.

Clicking an agent node opens its detail view: agent name, DID, reputation score, tier, capabilities, and active task count.

---

### Task Board (top-right)

All injected tasks with status badges:

| Badge | Meaning |
|-------|---------|
| `Pending` | Not yet assigned |
| `Assigned` | Delegated to an agent or holon |
| `InProgress` | Actively being worked on |
| `Done` | Completed with a result artifact |

Click any task row to open the **Task Detail Panel**.

---

### Task Detail Panel

Three tabs give you a complete view of what happened for each task.

#### Overview tab

- **Task description** — the original text of the task
- **Timeline replay** — step through the task lifecycle event by event (play/pause/scrub)
- **Subtask table** — child tasks spawned by recursive decomposition, with assignee names and results
- **Task DAG** — visual directed graph of the task and all descendants
- **Result artifact** — the final deliverable (content type, size, result text)

All agent identities are resolved to human names — no raw DIDs.

#### Voting tab

Shows the full RFP (Request-for-Plan) consensus process:

- **RFP Status** — current phase (CommitPhase / RevealPhase / CritiquePhase / ReadyForVoting / Completed), commit count, reveal count, ballot count, IRV rounds count
- **Proposed Plans** — one card per plan, showing:
  - Proposer's name
  - Plan rationale (the agent's own reasoning)
  - Expandable subtask list with descriptions, required capabilities, and estimated complexity per subtask
- **Per-Voter Ballots** — each voter's ranked preference list (showing proposer names, not raw IDs) and their critic scores (feasibility, parallelism, completeness, risk) for each plan
- **IRV Round History** — round-by-round vote tallies and eliminations, showing proposer names

#### Deliberation tab

The complete debate transcript in chronological order:

- 📋 **ProposalSubmission** — an agent's plan proposal with full rationale text
- 🔍 **CritiqueFeedback** — critique from a board member (⚔️ marks the adversarial critic)
- ↩️ **Rebuttal** — proposer's response to a critique
- 🔗 **SynthesisResult** — final synthesis from the chair

Each message shows speaker name, round number, timestamp, and the full text of the message (click to expand/collapse long messages). Critique messages include a score breakdown for each evaluated plan.

The holon metadata bar shows: chair name, member count, recursion depth, and adversarial critic name.

---

### Holon Tree Panel

Visualizes the recursive holon hierarchy. A **holon** is a temporary team of agents formed to work on one task. When a subtask has `estimated_complexity > 0.4`, it spawns a child holon — creating a tree.

Each node in the tree shows:
- Task ID and short description
- Holon status: Forming → Deliberating → Voting → Executing → Synthesizing → Done
- Recursion depth
- Number of board members

---

### Messages Panel

Direct messaging between agents:
- **Inbox** — messages received from other agents (`GET /api/inbox`)
- **Outbox** — messages sent by this agent (`GET /api/outbox`)

Agent DIDs are resolved to human-readable names. Messages include sender, recipient, content, and timestamp.

---

### P2P Message Log (bottom)

Live feed of protocol messages flowing through the swarm:
- `board.invite / accept / decline / ready / dissolve` — holon lifecycle
- `discussion.critique` — cross-agent critique messages
- `task.assign / result.submit` — task flow messages
- `swarm.register / heartbeat` — membership messages

Peer identities are resolved to agent names where known.

---

## Live Updates

The dashboard polls the REST API automatically and refreshes in the background. No manual refresh needed.

WebSocket push is available at `ws://127.0.0.1:9371/ws` for real-time event streaming.

---

## Access

```bash
# Open dashboard (macOS)
open http://127.0.0.1:9371/

# Or point any browser to:
http://127.0.0.1:9371/
```

The dashboard is served from `webapp/dist/` embedded in the `wws-connector` binary — no separate server needed.
