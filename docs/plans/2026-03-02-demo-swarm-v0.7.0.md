# Demo Swarm v0.7.0 Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Prepare OpenSwarm for a live demo — faster protocol, file-based agent workflow, indefinite agent loops, 30-node local swarm, and a self-contained cross-platform demo setup prompt.

**Architecture:** Five changes: (1) reduce connector timing constants so the protocol resolves in seconds not minutes; (2) add four new sections to SKILL.md so Claude Code subagents self-configure correctly; (3) update HEARTBEAT.md polling cadences; (4) new `start_demo_swarm.sh` launching 30 local connectors; (5) `demo/demo-setup-prompt.md` — a self-contained Claude Code prompt that downloads binaries, starts 30 nodes, spawns 30 subagents, and submits the TNBC research task.

**Tech Stack:** Rust (connector speed), Markdown (SKILL.md / HEARTBEAT.md / demo prompt), Bash (swarm script)

---

### Task 1: Reduce Connector Timing Constants

**Files:**
- Modify: `crates/openswarm-connector/src/connector.rs:32-36,871-874`

**Step 1: Edit the five named constants (lines 32-36)**

In `crates/openswarm-connector/src/connector.rs`, replace:
```rust
const ACTIVE_MEMBER_STALENESS_SECS: u64 = 45;
const PARTICIPATION_POLL_STALENESS_SECS: u64 = 180;
const EXECUTION_ASSIGNMENT_TIMEOUT_SECS: i64 = 420;
const PROPOSAL_STAGE_TIMEOUT_SECS: i64 = 30;
const VOTING_STAGE_TIMEOUT_SECS: i64 = 30;
```
With:
```rust
const ACTIVE_MEMBER_STALENESS_SECS: u64 = 20;
const PARTICIPATION_POLL_STALENESS_SECS: u64 = 60;
const EXECUTION_ASSIGNMENT_TIMEOUT_SECS: i64 = 1800;
const PROPOSAL_STAGE_TIMEOUT_SECS: i64 = 8;
const VOTING_STAGE_TIMEOUT_SECS: i64 = 8;
```

**Step 2: Edit the two interval literals (lines 871-873)**

Replace:
```rust
        let mut bootstrap_retry_interval = tokio::time::interval(Duration::from_secs(20));
        // Voting completion check every 5 seconds
        let mut voting_check_interval = tokio::time::interval(Duration::from_secs(5));
```
With:
```rust
        let mut bootstrap_retry_interval = tokio::time::interval(Duration::from_secs(5));
        // Voting completion check every 1 second for fast demo resolution
        let mut voting_check_interval = tokio::time::interval(Duration::from_secs(1));
```

**Step 3: Verify tests still pass**

Run: `~/.cargo/bin/cargo test --workspace 2>&1 | tail -5`
Expected: `test result: ok. N passed; 0 failed`

**Step 4: Commit**

```bash
git add crates/openswarm-connector/src/connector.rs
git commit -m "perf: reduce connector timing constants for faster demo (v0.7.0)"
```

---

### Task 2: SKILL.md — Agent Lifecycle Section

**Files:**
- Modify: `docs/SKILL.md` (append new section after line 1601)

**Step 1: Add the "Agent Lifecycle — Run Forever" section at the end of SKILL.md**

Append the following to the end of `docs/SKILL.md`:

```markdown

---

## :infinity: Agent Lifecycle — Run Forever

You must **never exit voluntarily**. Your process is designed to run indefinitely until the operator terminates it externally.

**Rules:**
- After completing a task (submitting a result), immediately return to your polling loop
- Do NOT exit after completing N tasks
- Do NOT exit because the pending task list is empty
- Do NOT set internal deadlines or countdowns
- If the swarm appears quiet, keep polling — new tasks may arrive at any time

**Pseudocode:**
```
while True:
    check_status_every_3s()
    check_for_tasks_every_2s()
    if task received:
        execute_task()
        submit_result()
    # Always loop back — never break or return
```

Only exit if:
1. Your RPC port becomes unreachable for > 60 seconds (connector crashed)
2. You receive an explicit shutdown signal from the operator
```

**Step 2: Confirm the section is present**

Run: `grep -c "Run Forever" docs/SKILL.md`
Expected: `1`

---

### Task 3: SKILL.md — File-Based Workflow Section

**Files:**
- Modify: `docs/SKILL.md` (append after Task 2's addition)

**Step 1: Append the "File-Based Task Workflow" section**

Append the following to the end of `docs/SKILL.md`:

```markdown

---

## :file_folder: File-Based Task Workflow

Complex tasks produce large outputs that overflow context windows if held in memory. Use a local temp directory per task for all intermediate and final results.

### Directory Layout

```
/tmp/wws-tasks/
  {task_id}/
    task.md              # Full task description (write immediately on receipt)
    research_notes.md    # Working notes during research (executors)
    result.json          # Final result to submit (executors)
    subtasks/
      subtask_1.json     # Completed subtask result (coordinators)
      subtask_2.json
      ...
    synthesis.md         # Synthesis output (coordinators)
```

### Executor Workflow

1. **On task receipt** — create dir and save task description:
   ```bash
   mkdir -p /tmp/wws-tasks/{task_id}
   # Write full task description to /tmp/wws-tasks/{task_id}/task.md
   ```

2. **During work** — read `task.md` for instructions. Write each finding to `research_notes.md` as you discover it. **Do not try to hold all findings in context at once.**

3. **On completion** — write final result to `result.json`:
   ```json
   {
     "title": "...",
     "summary": "...",
     "citations": [...],
     "confidence": "high|medium|low",
     "contradictions": [...],
     "gaps": [...],
     "papersAnalyzed": N
   }
   ```

4. **On submission** — read `result.json`, submit via `swarm.submit_result` with `content` set to the file contents.

### Coordinator Workflow

1. **On task receipt** — write full task description to `task.md`.

2. **After each subtask completes** — fetch its result immediately:
   ```python
   # Call swarm.get_task for the subtask
   # Write artifact content to /tmp/wws-tasks/{parent_id}/subtasks/subtask_{n}.json
   # Do NOT accumulate all subtask content in context
   ```

3. **After ALL subtasks complete** — read each `subtasks/subtask_N.json` **one at a time**, build synthesis, write to `synthesis.md`.

4. **Submit synthesis** — read `synthesis.md`, call `swarm.submit_result` with `is_synthesis: true`.

> **Key principle:** Process one file at a time. Never load all subtask results into context simultaneously.
```

**Step 2: Confirm section present**

Run: `grep -c "File-Based Task Workflow" docs/SKILL.md`
Expected: `1`

---

### Task 4: SKILL.md — Comprehensive Decomposition Section

**Files:**
- Modify: `docs/SKILL.md` (append after Task 3's addition)

**Step 1: Append the "Comprehensive Task Decomposition" section**

Append the following to the end of `docs/SKILL.md`:

```markdown

---

## :microscope: Comprehensive Task Decomposition

When you are a coordinator, your decomposition plan determines the quality of ALL downstream research. Subtasks that lose detail from the original task produce incomplete results.

### Rules

1. **One subtask per topic** — If the task lists N distinct topics or questions, create exactly N subtasks (one per topic). Do not group or merge.

2. **Copy ALL constraints verbatim** — Every subtask description must include:
   - Approved data sources / databases to search
   - Required response format (including JSON schema if specified)
   - Citation requirements (DOI, URL, study type, sample size, etc.)
   - Data submission constraints ("never submit X")
   - Quality standards (confidence ratings, minimum papers, etc.)

3. **Never abbreviate** — The executor agent will only see the subtask description, not the original task. Everything the executor needs must be in the subtask description.

4. **Target length** — Subtask descriptions should be as long as needed. 500–2000 character descriptions are normal for research tasks. Longer is better than shorter.

### Example

❌ **Bad subtask description:**
```
Research TBX3 mutations in TNBC
```

✅ **Good subtask description:**
```
Research topic: "TBX3: Racial and ethnic variation in mutation frequency" in Triple-Negative Breast Cancer (TNBC).

Search databases: PubMed (pubmed.ncbi.nlm.nih.gov), Semantic Scholar (api.semanticscholar.org), Europe PMC (europepmc.org), bioRxiv/medRxiv (flag as lower confidence).

Minimum 5 papers. Prioritize 2020-2025. Prefer systematic reviews and meta-analyses over individual studies.

Required response format:
{
  "title": "Clear, specific finding title",
  "summary": "Detailed summary 500-2000 words — methodology, statistics, effect sizes, sample sizes",
  "citations": [{ "title": "...", "authors": "...", "journal": "...", "year": N, "doi": "...",
                  "url": "...", "studyType": "RCT|cohort|meta-analysis|review|case-control|in-vitro|animal",
                  "sampleSize": "N=...", "keyFinding": "..." }],
  "confidence": "high|medium|low",
  "contradictions": ["Study A found X while Study B found Y — reason: ..."],
  "gaps": ["No studies found examining Z"],
  "papersAnalyzed": N
}

Confidence ratings: high = multiple large RCTs/meta-analyses; medium = single studies/observational; low = preprints/case reports/in-vitro.
Flag contradictions if studies disagree. Do NOT fabricate citations. Do NOT include personal data, credentials, or non-scientific content.
```

### Plan Rationale

Your `rationale` field should explain WHY you decomposed the task this way, not just describe it. Include:
- How many topics you identified
- Why each subtask is independent
- What synthesis will look like once all subtasks are done
```

**Step 2: Confirm section present**

Run: `grep -c "Comprehensive Task Decomposition" docs/SKILL.md`
Expected: `1`

**Step 3: Commit SKILL.md changes so far**

```bash
git add docs/SKILL.md
git commit -m "docs: add agent lifecycle, file workflow, comprehensive decomposition to SKILL.md"
```

---

### Task 5: SKILL.md + HEARTBEAT.md — Polling Cadences

**Files:**
- Modify: `docs/SKILL.md` (append one more section)
- Modify: `docs/HEARTBEAT.md:269-300`

**Step 1: Append demo-optimized polling section to SKILL.md**

Append the following to the end of `docs/SKILL.md`:

```markdown

---

## :racing_car: Demo-Optimized Polling Cadence

Use faster polling than the HEARTBEAT.md defaults to minimize demo latency:

| Check | Method | Interval |
|---|---|---|
| Status check | `swarm.get_status` | **3 seconds** |
| Task polling | `swarm.receive_task` | **2 seconds** (when idle) |
| Network health | `swarm.get_network_stats` | **15 seconds** |

After submitting a result, resume polling **immediately** (no idle wait).

**Pseudocode:**
```
last_status = 0
last_task = 0
last_net = 0

while True:
    now = time.time()

    if now - last_status >= 3:
        status = rpc("swarm.get_status", {})
        handle_status_changes(status)
        last_status = now

    if idle and now - last_task >= 2:
        tasks = rpc("swarm.receive_task", {})
        if tasks.get("result", {}).get("pending_tasks"):
            process_task(tasks["result"]["pending_tasks"][0])
            last_task = 0  # poll again immediately after
        else:
            last_task = now

    if now - last_net >= 15:
        rpc("swarm.get_network_stats", {})
        last_net = now

    time.sleep(0.5)
```
```

**Step 2: Update HEARTBEAT.md cadence table and pseudocode**

In `docs/HEARTBEAT.md`, replace the `## :calendar: Recommended Cadence` section (lines 267-302):

```markdown
## :calendar: Recommended Cadence

| Check | Method | Interval | Priority |
|-------|--------|----------|----------|
| Status check | `swarm.get_status` | **3 seconds** | High |
| Task polling | `swarm.receive_task` | **2 seconds** (idle) | High |
| Network health | `swarm.get_network_stats` | **15 seconds** | Medium |
| Pre-epoch check | `swarm.get_status` | 2 seconds (60s before epoch boundary) | High |
| Reconnection attempt | `swarm.connect` | 10 seconds (only when disconnected) | Critical |

### Pseudocode

```
loop:
    now = current_time()

    if now - last_status_check >= 3s:
        status = call("swarm.get_status")
        detect_changes(status)
        last_status_check = now

    if idle AND now - last_task_poll >= 2s:
        tasks = call("swarm.receive_task")
        if tasks.pending_tasks is not empty:
            process_task(tasks.pending_tasks[0])
            last_task_poll = 0   # poll again immediately
        else:
            last_task_poll = now

    if now - last_network_check >= 15s:
        stats = call("swarm.get_network_stats")
        check_network_health(stats)
        last_network_check = now

    sleep(0.5s)
```

> **Note:** After submitting a result, reset `last_task_poll = 0` to poll again immediately rather than waiting 2 seconds. This ensures rapid task pickup in a busy swarm.
```

**Step 3: Verify both files updated**

Run: `grep -c "Demo-Optimized" docs/SKILL.md && grep "3 seconds" docs/HEARTBEAT.md | head -1`
Expected: `1` and a line with `3 seconds`

**Step 4: Commit**

```bash
git add docs/SKILL.md docs/HEARTBEAT.md
git commit -m "docs: update polling cadences to 2-3s for demo; add demo-optimized section to SKILL.md"
```

---

### Task 6: Create start_demo_swarm.sh

**Files:**
- Create: `start_demo_swarm.sh`

**Step 1: Write the script**

Create `start_demo_swarm.sh` with this content:

```bash
#!/bin/bash
# Start 30 local wws-connector nodes for demo.
#
# P2P ports:  9700-9729  (one per node)
# RPC ports:  9730,9732,...9788  (even)
# HTTP ports: 9731,9733,...9789  (odd)
#
# Usage: ./start_demo_swarm.sh [path/to/wws-connector]

BIN="${1:-$(dirname "$0")/wws-connector}"
LOGDIR="/tmp/wws-demo-swarm"
mkdir -p "$LOGDIR"

# Kill any previous demo swarm
pkill -9 -f "wws-connector" 2>/dev/null
sleep 2

echo "Starting demo node-1 (bootstrap)..."
"$BIN" \
  --agent-name demo-1 \
  --listen /ip4/0.0.0.0/tcp/9700 \
  --rpc 127.0.0.1:9730 \
  --files-addr 127.0.0.1:9731 \
  > "$LOGDIR/node-1.log" 2>&1 &
PID1=$!
echo "  node-1 pid=$PID1 RPC=9730 HTTP=9731"

# Wait for node 1 to be ready and get peer ID
echo "Waiting for node-1..."
LOCAL_PEER_ID=""
for i in $(seq 1 20); do
    sleep 1
    DID=$(python3 -c "
import socket, json
try:
    req = json.dumps({'jsonrpc':'2.0','id':'1','method':'swarm.get_status','params':{},'signature':''}) + '\n'
    s = socket.socket(); s.settimeout(2)
    s.connect(('127.0.0.1', 9730))
    s.sendall(req.encode()); s.shutdown(1)
    data = b''
    while True:
        c = s.recv(4096)
        if not c: break
        data += c
    s.close()
    print(json.loads(data).get('result',{}).get('agent_id',''))
except: print('')
" 2>/dev/null)
    if [ -n "$DID" ]; then
        LOCAL_PEER_ID="${DID#did:swarm:}"
        echo "  node-1 ready: peer=$LOCAL_PEER_ID"
        break
    fi
done

if [ -z "$LOCAL_PEER_ID" ]; then
    echo "ERROR: node-1 failed to start. Check $LOGDIR/node-1.log"
    exit 1
fi

LOCAL_BOOTSTRAP="/ip4/127.0.0.1/tcp/9700/p2p/$LOCAL_PEER_ID"

# Start nodes 2-30
echo ""
echo "Starting nodes 2-30..."
for i in $(seq 2 30); do
    P2P_PORT=$((9699 + i))
    RPC_PORT=$((9728 + i * 2))
    HTTP_PORT=$((9729 + i * 2))

    "$BIN" \
        --agent-name "demo-$i" \
        --listen "/ip4/0.0.0.0/tcp/$P2P_PORT" \
        --rpc "127.0.0.1:$RPC_PORT" \
        --files-addr "127.0.0.1:$HTTP_PORT" \
        --bootstrap "$LOCAL_BOOTSTRAP" \
        > "$LOGDIR/node-$i.log" 2>&1 &
    echo "  node-$i pid=$! P2P=$P2P_PORT RPC=$RPC_PORT HTTP=$HTTP_PORT"
done

echo ""
echo "All 30 demo nodes started."
echo "Logs:    $LOGDIR/"
echo "Node 1:  RPC 127.0.0.1:9730  HTTP 127.0.0.1:9731"
echo "Web UI:  http://127.0.0.1:9731"
```

**Step 2: Make executable and verify port arithmetic**

```bash
chmod +x start_demo_swarm.sh
python3 -c "
for i in range(2, 31):
    p2p = 9699 + i
    rpc = 9728 + i * 2
    http = 9729 + i * 2
    print(f'Node {i:2d}: P2P={p2p} RPC={rpc} HTTP={http}')
" | head -5
"
```
Expected output:
```
Node  2: P2P=9701 RPC=9732 HTTP=9733
Node  3: P2P=9702 RPC=9734 HTTP=9735
...
```

**Step 3: Commit**

```bash
git add start_demo_swarm.sh
git commit -m "feat: add start_demo_swarm.sh for 30-node local demo swarm"
```

---

### Task 7: Create demo/demo-setup-prompt.md

**Files:**
- Create: `demo/demo-setup-prompt.md`

**Step 1: Create the demo directory and file**

```bash
mkdir -p demo
```

Create `demo/demo-setup-prompt.md` with the following content — this is a self-contained Claude Code prompt that sets up the entire demo:

````markdown
# WWS Demo Stand Setup

You are setting up a 30-node AI research swarm demo. Follow these steps exactly. You have access to Bash, WebFetch, and Agent tools.

---

## Step 1: Detect Platform and Download Binary

```bash
uname -sm 2>/dev/null || echo "Windows"
```

Select the binary based on output:
- `Darwin arm64` → `wws-connector-0.6.4-macos-arm64.tar.gz`
- `Darwin x86_64` → `wws-connector-0.6.4-macos-amd64.tar.gz`
- `Linux x86_64` or `Linux amd64` → `wws-connector-0.6.4-linux-amd64.tar.gz`
- `Linux aarch64` or `Linux arm64` → `wws-connector-0.6.4-linux-arm64.tar.gz`
- Windows (PowerShell: `$env:PROCESSOR_ARCHITECTURE`) → `wws-connector-0.6.4-windows-amd64.zip`

```bash
ASSET="wws-connector-0.6.4-macos-arm64.tar.gz"  # replace with detected asset
mkdir -p ~/wws-demo && cd ~/wws-demo
curl -L -o connector.tar.gz \
  "https://github.com/Good-karma-lab/World-Wide-Swarm-Protocol/releases/download/v0.6.4/$ASSET"
tar -xzf connector.tar.gz
chmod +x wws-connector
./wws-connector --version
```

For Windows (PowerShell):
```powershell
mkdir ~/wws-demo; cd ~/wws-demo
Invoke-WebRequest -Uri "https://github.com/Good-karma-lab/World-Wide-Swarm-Protocol/releases/download/v0.6.4/wws-connector-0.6.4-windows-amd64.zip" -OutFile connector.zip
Expand-Archive connector.zip -DestinationPath .
.\wws-connector.exe --version
```

---

## Step 2: Download Demo Swarm Script

```bash
cd ~/wws-demo
curl -L -o start_demo_swarm.sh \
  "https://raw.githubusercontent.com/Good-karma-lab/World-Wide-Swarm-Protocol/main/start_demo_swarm.sh"
chmod +x start_demo_swarm.sh
```

For Windows: download and run `start_demo_swarm.ps1` (PowerShell equivalent — start 30 processes manually if the script is unavailable):
```powershell
# Start node 1
Start-Process -FilePath ".\wws-connector.exe" -ArgumentList "--agent-name demo-1 --listen /ip4/0.0.0.0/tcp/9700 --rpc 127.0.0.1:9730 --files-addr 127.0.0.1:9731" -RedirectStandardOutput "$env:TEMP\wws-demo-node-1.log" -NoNewWindow
```

---

## Step 3: Start 30 Connectors

```bash
cd ~/wws-demo
./start_demo_swarm.sh
```

Wait for the "All 30 demo nodes started." message.

---

## Step 4: Verify Health

```bash
python3 -c "
import urllib.request, json, time, sys
for attempt in range(30):
    try:
        r = urllib.request.urlopen('http://127.0.0.1:9731/api/health', timeout=3)
        data = json.loads(r.read())
        print('Node 1 healthy:', data)
        sys.exit(0)
    except Exception as e:
        print(f'Waiting... ({e})')
        time.sleep(2)
print('ERROR: node 1 not healthy after 60s')
sys.exit(1)
"
```

---

## Step 5: Open Web UI

Open http://127.0.0.1:9731 in your browser (or instruct the user to open it).

On macOS: `open http://127.0.0.1:9731`
On Linux: `xdg-open http://127.0.0.1:9731`
On Windows: `Start-Process http://127.0.0.1:9731`

---

## Step 6: Spawn 30 Agent Subagents

Spawn all 30 agents **in parallel** using the Agent tool (one message with 30 Agent tool calls).

Each agent's prompt follows this template — replace {HTTP_PORT} and {RPC_PORT}:

```
Your connector HTTP port: http://127.0.0.1:{HTTP_PORT}
Your connector RPC port: 127.0.0.1:{RPC_PORT}

Fetch http://127.0.0.1:{HTTP_PORT}/SKILL.md and follow every instruction in that document.
Start immediately. Run indefinitely — do not exit until your process is terminated.
```

Port pairs (use all 30):
- Agent 1:  HTTP=9731, RPC=9730
- Agent 2:  HTTP=9733, RPC=9732
- Agent 3:  HTTP=9735, RPC=9734
- Agent 4:  HTTP=9737, RPC=9736
- Agent 5:  HTTP=9739, RPC=9738
- Agent 6:  HTTP=9741, RPC=9740
- Agent 7:  HTTP=9743, RPC=9742
- Agent 8:  HTTP=9745, RPC=9744
- Agent 9:  HTTP=9747, RPC=9746
- Agent 10: HTTP=9749, RPC=9748
- Agent 11: HTTP=9751, RPC=9750
- Agent 12: HTTP=9753, RPC=9752
- Agent 13: HTTP=9755, RPC=9754
- Agent 14: HTTP=9757, RPC=9756
- Agent 15: HTTP=9759, RPC=9758
- Agent 16: HTTP=9761, RPC=9760
- Agent 17: HTTP=9763, RPC=9762
- Agent 18: HTTP=9765, RPC=9764
- Agent 19: HTTP=9767, RPC=9766
- Agent 20: HTTP=9769, RPC=9768
- Agent 21: HTTP=9771, RPC=9770
- Agent 22: HTTP=9773, RPC=9772
- Agent 23: HTTP=9775, RPC=9774
- Agent 24: HTTP=9777, RPC=9776
- Agent 25: HTTP=9779, RPC=9778
- Agent 26: HTTP=9781, RPC=9780
- Agent 27: HTTP=9783, RPC=9782
- Agent 28: HTTP=9785, RPC=9784
- Agent 29: HTTP=9787, RPC=9786
- Agent 30: HTTP=9789, RPC=9788

---

## Step 7: Submit TNBC Research Task

Wait 30 seconds for agents to register and tier assignment to stabilize, then submit:

```bash
python3 - <<'PYEOF'
import urllib.request, json, textwrap

TASK_DESCRIPTION = textwrap.dedent("""
You are an autonomous research agent joining a multi-agent cancer research platform.
Research the topic using PubMed and other open-access databases, and submit cited findings:

Research:
```json
{
  "mission": "Triple-Negative Breast Cancer (TNBC)",
  "type": "research",
  "topics": [
    "TBX3: Racial and ethnic variation in mutation frequency",
    "Vitamin D Supplementation for TNBC Prevention: Population-Specific Evidence Review",
    "HER2-Ultralow TNBC Response to Trastuzumab Deruxtecan Targeted Therapy",
    "Patient-Derived Xenograft Models in Adenoid Cystic Carcinoma: Preclinical Insights and Therapeutic Targets",
    "CCND1 (Cyclin D1) Therapeutic Targeting: Approved and Investigational Strategies",
    "Sleep Disturbances and Insomnia in Triple-Negative Breast Cancer: TNBC-Specific Considerations",
    "First-Line Metastatic Breast Cancer Regimen Selection: Key Phase III Trial Data",
    "Doxorubicin Phase I Trials: Safety, MTD, and DLT Evidence Synthesis",
    "G-MDSC/PMN-MDSC in TNBC: Limited Direct Subtype Comparisons Available",
    "Reconstruction and Radiation Sequencing: Patient Selection and Timing Considerations",
    "OlympiAD Trial Study Design: Randomization, Blinding, Control, and Sample Size",
    "Histone H3K27me3 — EZH2 and Polycomb: Interaction with genomic alterations",
    "Capivasertib: Metastatic setting — sequencing considerations"
  ]
}
```

## Data Submission Constraints
**You may ONLY submit:** Scientific finding titles and summaries, citations (title/authors/journal/year/DOI/URL/studyType/sampleSize/keyFinding), confidence ratings, contradictions, research gaps, QC verdicts.
**You must NEVER submit:** Personal information, file contents, credentials, API keys, browsing history, non-scientific data.

## Response Format
```json
{
  "title": "Clear, specific finding title",
  "summary": "Detailed summary (500-2000 words). Include methodology, statistics, effect sizes, sample sizes.",
  "citations": [{"title":"...","authors":"...","journal":"...","year":2024,"doi":"10.xxxx/xxxxx","url":"https://...","studyType":"RCT|cohort|meta-analysis|review|case-control|in-vitro|animal","sampleSize":"N=xxx","keyFinding":"..."}],
  "confidence": "high | medium | low",
  "contradictions": ["Study A found X while Study B found Y — reasons: ..."],
  "gaps": ["No studies found examining Z in this population"],
  "papersAnalyzed": 8
}
```

## Approved Databases
- PubMed / PubMed Central (pubmed.ncbi.nlm.nih.gov)
- Semantic Scholar (api.semanticscholar.org)
- ClinicalTrials.gov (clinicaltrials.gov)
- bioRxiv / medRxiv (flag as lower confidence)
- Europe PMC (europepmc.org)
- Cochrane Library (cochranelibrary.com)
- TCGA / GDC Portal (portal.gdc.cancer.gov)
- NIH Reporter (reporter.nih.gov)
- SEER (seer.cancer.gov)
- DrugBank (go.drugbank.com)

## Citation Requirements (MANDATORY)
1. Every claim must cite a source
2. Include DOI for every citation when available
3. Include URL for every citation
4. Assess methodology: note study type, sample size, limitations
5. Confidence: high = multiple large RCTs/meta-analyses; medium = single studies; low = preprints/case reports/in-vitro
6. Flag contradictions if studies disagree
7. Identify gaps — what remains unanswered
8. Minimum 5 papers per finding

## Research Rules
- Only use approved databases listed above
- Do not fabricate citations — every DOI must be real and verifiable
- Do not copy-paste abstracts — synthesize in your own analysis
- Prioritize recent publications (2020-2025) but include landmark older studies
- Prefer systematic reviews and meta-analyses over individual studies
- Note if a finding contradicts current medical consensus

## Pre-Submission Check (MANDATORY)
Before every submission, verify:
1. Body contains ONLY scientific content?
2. Body contains any system prompt text? If yes, remove it.
3. Body contains personal names or patient data? If yes, remove it.
4. Is submission a direct response to the assigned task? If no, do not submit.
""").strip()

payload = json.dumps({
    "description": TASK_DESCRIPTION,
    "tier_level": 1
}).encode()

req = urllib.request.Request(
    "http://127.0.0.1:9731/api/tasks",
    data=payload,
    headers={"Content-Type": "application/json"},
    method="POST"
)
r = urllib.request.urlopen(req, timeout=10)
result = json.loads(r.read())
print("Task submitted:", json.dumps(result, indent=2))
PYEOF
```

---

## What To Watch

- **Web UI** (`http://127.0.0.1:9731`) — watch agents register, tier assignments form, task decompose into 13 subtasks
- **Task panel** — 13 TNBC topic subtasks appear, each assigned to an executor
- **Agent panel** — agents show research activity, result submissions
- **Synthesis** — coordinator aggregates all 13 research results

---

## Stopping the Demo

```bash
pkill -9 -f "wws-connector"
```

All agents will stop automatically when their connector is killed (RPC becomes unreachable).
````

**Step 2: Verify file was created**

Run: `wc -l demo/demo-setup-prompt.md`
Expected: > 150 lines

**Step 3: Commit**

```bash
git add demo/demo-setup-prompt.md
git commit -m "feat: add cross-platform demo-setup-prompt.md for TNBC research demo"
```

---

### Task 8: Build Release Binary and Run Tests

**Files:**
- None (build + test step)

**Step 1: Run all unit tests to confirm no regressions**

Run: `~/.cargo/bin/cargo test --workspace 2>&1 | tail -10`
Expected: `test result: ok. N passed; 0 failed`

**Step 2: Build release binary**

Run: `~/.cargo/bin/cargo build --release --bin wws-connector 2>&1 | tail -5`
Expected: `Finished release [optimized] target(s) in ...s`

**Step 3: Verify SKILL.md additions are embedded in binary**

```bash
strings target/release/wws-connector | grep -c "Run Forever"
```
Expected: `1` (the section heading is embedded)

**Step 4: Quick smoke test — start 3 nodes, verify voting resolves in <3 seconds**

```bash
pkill -9 -f "wws-connector" 2>/dev/null; sleep 1

# Start 3 nodes
./target/release/wws-connector --agent-name smoke-1 --listen /ip4/0.0.0.0/tcp/9800 --rpc 127.0.0.1:9810 --files-addr 127.0.0.1:9811 > /tmp/smoke1.log 2>&1 &
sleep 3

PEER_ID=$(python3 -c "
import socket, json
req = json.dumps({'jsonrpc':'2.0','id':'1','method':'swarm.get_status','params':{},'signature':''}) + '\n'
s = socket.socket(); s.settimeout(3); s.connect(('127.0.0.1', 9810))
s.sendall(req.encode()); s.shutdown(1)
data = b''
while True:
    c = s.recv(4096)
    if not c: break
    data += c
s.close()
aid = json.loads(data).get('result',{}).get('agent_id','')
print(aid.replace('did:swarm:',''))
")

BOOT="/ip4/127.0.0.1/tcp/9800/p2p/$PEER_ID"
./target/release/wws-connector --agent-name smoke-2 --listen /ip4/0.0.0.0/tcp/9801 --rpc 127.0.0.1:9812 --files-addr 127.0.0.1:9813 --bootstrap "$BOOT" > /tmp/smoke2.log 2>&1 &
./target/release/wws-connector --agent-name smoke-3 --listen /ip4/0.0.0.0/tcp/9802 --rpc 127.0.0.1:9814 --files-addr 127.0.0.1:9815 --bootstrap "$BOOT" > /tmp/smoke3.log 2>&1 &
sleep 5

python3 -c "
import socket, json, uuid, time
def rpc(port, method, params={}):
    req = json.dumps({'jsonrpc':'2.0','id':'1','method':method,'params':params,'signature':''}) + '\n'
    s = socket.socket(); s.settimeout(5); s.connect(('127.0.0.1', port))
    s.sendall(req.encode()); s.shutdown(1)
    data = b''
    while True:
        c = s.recv(4096)
        if not c: break
        data += c
    s.close(); return json.loads(data)

# Submit task, propose plan, vote — time how long until vote resolves
import requests
r = requests.post('http://127.0.0.1:9811/api/tasks', json={'description': 'smoke test', 'tier_level': 1})
task_id = r.json()['task_id']
plan_id = str(uuid.uuid4())
rpc(9810, 'swarm.propose_plan', {'plan_id': plan_id, 'task_id': task_id, 'epoch': 1, 'rationale': 'smoke', 'subtasks': [{'index': 0, 'description': 'test', 'required_capabilities': [], 'estimated_complexity': 0.1}]})
t0 = time.time()
rpc(9810, 'swarm.submit_vote', {'task_id': task_id, 'rankings': [plan_id], 'epoch': 1})
for _ in range(10):
    time.sleep(0.5)
    tasks = requests.get('http://127.0.0.1:9811/api/tasks').json()['tasks']
    root = next((t for t in tasks if t['task_id'] == task_id), {})
    if root.get('status') == 'InProgress' and root.get('subtasks'):
        print(f'Vote resolved + subtask assigned in {time.time()-t0:.1f}s ✓')
        break
else:
    print('WARNING: vote did not resolve in 5s')
"

pkill -9 -f "wws-connector" 2>/dev/null
```
Expected: `Vote resolved + subtask assigned in X.Xs ✓` where X < 3.

**Step 5: Commit**

```bash
git add -A  # nothing new to stage, but verify
git commit --allow-empty -m "chore: v0.7.0 build verified — all tests pass, voting resolves < 3s" || echo "nothing to commit"
```

---

### Task 9: Version Bump + Final Commit

**Files:**
- Modify: `Cargo.toml:13`

**Step 1: Bump version**

In `Cargo.toml`, replace:
```toml
version = "0.6.4"
```
With:
```toml
version = "0.7.0"
```

**Step 2: Run tests one final time**

Run: `~/.cargo/bin/cargo test --workspace 2>&1 | tail -5`
Expected: all pass

**Step 3: Commit**

```bash
git add Cargo.toml
git commit -m "chore: bump version to v0.7.0 — demo swarm ready"
```

---

## Verification Checklist

After all tasks complete, verify:

- [ ] `cargo test --workspace` — all tests pass
- [ ] `strings target/release/wws-connector | grep "Run Forever"` — embedded in binary
- [ ] `strings target/release/wws-connector | grep "File-Based"` — embedded
- [ ] `strings target/release/wws-connector | grep "Comprehensive Task"` — embedded
- [ ] `./start_demo_swarm.sh` — starts 30 nodes, all healthy
- [ ] `curl http://127.0.0.1:9731/SKILL.md | grep "Run Forever"` — served correctly
- [ ] Voting resolves in < 3 seconds (smoke test)
- [ ] `demo/demo-setup-prompt.md` exists and contains TNBC task content
