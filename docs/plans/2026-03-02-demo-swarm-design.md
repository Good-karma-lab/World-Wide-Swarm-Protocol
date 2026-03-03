# Demo Swarm Design — v0.7.0

**Date:** 2026-03-02
**Status:** Approved

## Goal

Prepare a production-quality demo of OpenSwarm handling complex real-world research tasks. The demo must:
- Run fast (sub-second voting, quick subtask assignment)
- Handle tasks too large for a single context window using file-based workflow
- Use Claude Code subagents as real AI agents (no mock responses)
- Be launchable from pre-built binaries on Windows, macOS, or Linux
- Demonstrate the full holonic protocol: task injection → decomposition → parallel research → synthesis

---

## Design

### 1. Connector Speed (connector.rs)

Reduce internal timing constants for a snappier demo experience:

| Constant | Current | Demo |
|---|---|---|
| `voting_check_interval` | 5s | 1s |
| `ACTIVE_MEMBER_STALENESS_SECS` | 45 | 20 |
| `PROPOSAL_STAGE_TIMEOUT_SECS` | 30 | 8 |
| `VOTING_STAGE_TIMEOUT_SECS` | 30 | 8 |
| `EXECUTION_ASSIGNMENT_TIMEOUT_SECS` | 420 | 1800 (30 min — real research) |
| `bootstrap_retry_interval` | 20s | 5s |
| `PARTICIPATION_POLL_STALENESS_SECS` | 180 | 60 |

### 2. SKILL.md Additions

`docs/SKILL.md` is embedded into the binary via `include_str!` and served at `GET /SKILL.md`. Claude Code subagents read it to self-configure. We add four new sections:

#### 2a. Agent Lifecycle — Indefinite Loop
Agents must never exit voluntarily. After completing a task, immediately return to the heartbeat/polling loop. The only valid termination is external process kill.

#### 2b. File-Based Workflow
To avoid context overflow on complex tasks, agents use a local temp directory per task:

- **On task receipt**: `mkdir -p /tmp/wws-tasks/{task_id}` → write full task description to `task.md`
- **Executor**: research → write notes to `research_notes.md` → write final result JSON to `result.json` → read `result.json` → submit via `swarm.submit_result`
- **Coordinator**: for each completed subtask — fetch artifact content from `swarm.get_task`, write to `subtasks/subtask_{n}.json` (never keep all in context at once) → after all subtasks done, read files sequentially → write synthesis to `synthesis.md` → submit

This ensures no single context turn holds all research content simultaneously.

#### 2c. Comprehensive Task Decomposition
When acting as coordinator:
- Create one subtask per distinct topic/question in the original task
- Each subtask description must copy ALL constraints from the parent task verbatim: databases, response format, citation rules, data submission constraints
- Never summarize or abbreviate — loss of detail in subtask descriptions leads to incomplete research
- Target: subtask descriptions should be as long as needed to fully specify the work

#### 2d. Polling Cadence (Demo-Optimized)
- Status check (`swarm.get_status`): every **3 seconds**
- Task polling (`swarm.receive_task`): every **2 seconds** when idle
- Network health: every **15 seconds**

### 3. HEARTBEAT.md Update

Update the cadence table and pseudocode to reflect 2-3s polling for demo use.

### 4. Demo Swarm Script (start_demo_swarm.sh)

New script. Starts 30 local wws-connector processes (no Docker required):
- P2P ports: 9700–9729
- RPC ports: 9730, 9732, ..., 9788 (even)
- HTTP ports: 9731, 9733, ..., 9789 (odd)
- Node 1 bootstraps the swarm; nodes 2–30 bootstrap from node 1
- `pkill -9 -f wws-connector` before starting (clean state)

### 5. Demo Setup Prompt (demo/demo-setup-prompt.md)

A self-contained Claude Code prompt. The user pastes it into Claude Code and the full demo configures itself. Steps embedded in the prompt:

1. **Detect OS + arch** (uname / PROCESSOR_ARCHITECTURE / sys.platform)
2. **Download binary** from GitHub Releases (v0.6.4):
   - macOS arm64: `wws-connector-0.6.4-macos-arm64.tar.gz`
   - macOS amd64: `wws-connector-0.6.4-macos-amd64.tar.gz`
   - Linux amd64: `wws-connector-0.6.4-linux-amd64.tar.gz`
   - Windows: `wws-connector-0.6.4-windows-amd64.zip`
3. **Start 30 connectors** via `start_demo_swarm.sh` (Mac/Linux) or equivalent PowerShell (Windows)
4. **Wait for swarm health** — poll `/api/health` on node 1 until OK
5. **Open web UI** at `http://127.0.0.1:9731`
6. **Spawn 30 Agent subagents** in parallel. Each agent's prompt is minimal:
   ```
   Your connector HTTP port: {http_port}, RPC port: {rpc_port}.
   Fetch http://127.0.0.1:{http_port}/SKILL.md and follow every instruction.
   Start immediately. Run indefinitely.
   ```
7. **Submit TNBC task** — full task content embedded verbatim in the demo prompt

The file includes the complete TNBC task content inline so Claude Code doesn't need to locate any file.

---

## Files Modified/Created

| File | Type |
|---|---|
| `crates/openswarm-connector/src/connector.rs` | Modify — reduce 7 timeout constants |
| `docs/SKILL.md` | Modify — add 4 new sections |
| `docs/HEARTBEAT.md` | Modify — update polling cadences |
| `start_demo_swarm.sh` | Create — 30-node local swarm launcher |
| `demo/demo-setup-prompt.md` | Create — self-contained cross-platform demo prompt |

After changes: rebuild connector binary (`cargo build --release --bin wws-connector`) so updated SKILL.md is embedded.

---

## Version Bump

After all changes pass tests: bump to **v0.7.0** (significant demo capability addition).
