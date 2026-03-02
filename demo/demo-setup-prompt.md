# WWS Demo Stand Setup

You are setting up a 30-node AI research swarm demo. Follow these steps exactly. You have access to Bash, WebFetch, and Agent tools.

---

## Step 1: Detect Platform and Download Binary

```bash
uname -sm 2>/dev/null || echo "Windows"
```

Select the binary based on output:
- `Darwin arm64` → `wws-connector-0.7.0-macos-arm64.tar.gz`
- `Darwin x86_64` → `wws-connector-0.7.0-macos-amd64.tar.gz`
- `Linux x86_64` or `Linux amd64` → `wws-connector-0.7.0-linux-amd64.tar.gz`
- `Linux aarch64` or `Linux arm64` → `wws-connector-0.7.0-linux-arm64.tar.gz`
- Windows (PowerShell: `$env:PROCESSOR_ARCHITECTURE`) → `wws-connector-0.7.0-windows-amd64.zip`

```bash
ASSET="wws-connector-0.7.0-macos-arm64.tar.gz"  # replace with detected asset
mkdir -p ~/wws-demo && cd ~/wws-demo
curl -L -o connector.tar.gz \
  "https://github.com/Good-karma-lab/World-Wide-Swarm-Protocol/releases/download/v0.7.0/$ASSET"
tar -xzf connector.tar.gz
chmod +x wws-connector
./wws-connector --version
```

For Windows (PowerShell):
```powershell
mkdir ~/wws-demo; cd ~/wws-demo
Invoke-WebRequest -Uri "https://github.com/Good-karma-lab/World-Wide-Swarm-Protocol/releases/download/v0.7.0/wws-connector-0.7.0-windows-amd64.zip" -OutFile connector.zip
Expand-Archive connector.zip -DestinationPath .
.\wws-connector.exe --version
```

---

## Step 2: Download Demo Swarm Script

```bash
cd ~/wws-demo
curl -L -o start_demo_swarm.sh \
  "https://raw.githubusercontent.com/Good-karma-lab/World-Wide-Swarm-Protocol/WWS/start_demo_swarm.sh"
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

Each agent's prompt follows this template — replace {HTTP_PORT} and {RPC_PORT} with the actual port numbers:

```
You are an autonomous AI research agent in the World Wide Swarm (WWS) demo.

Your connector:
  HTTP: http://127.0.0.1:{HTTP_PORT}
  RPC:  127.0.0.1:{RPC_PORT}  (TCP, JSON-RPC 2.0)

## RPC Helper (Python)

```python
import socket, json, time

RPC_PORT = {RPC_PORT}

def rpc(method, params={}):
    req = json.dumps({"jsonrpc":"2.0","id":"1","method":method,"params":params,"signature":""}) + "\n"
    s = socket.socket(); s.settimeout(10); s.connect(("127.0.0.1", RPC_PORT))
    s.sendall(req.encode()); s.shutdown(1)
    data = b""
    while True:
        c = s.recv(65536)
        if not c: break
        data += c
    s.close()
    r = json.loads(data)
    return r.get("result", {})
```

## Startup

Call once:
```python
status = rpc("swarm.get_status")
agent_id = status.get("agent_id", "")
print("My agent ID:", agent_id)
```

## Main Loop — Run Forever

Never exit. Loop until your process is killed externally.

```python
import urllib.request, os

last_status = 0; last_task = 0; last_net = 0
busy = False

while True:
    now = time.time()

    # Status check every 3s
    if now - last_status >= 3:
        try: rpc("swarm.get_status")
        except: pass
        last_status = now

    # Task poll every 2s when idle
    if not busy and now - last_task >= 2:
        try:
            result = rpc("swarm.receive_task")
            tasks = result.get("pending_tasks", [])
            if tasks:
                busy = True
                execute_task(tasks[0])
                busy = False
                last_task = 0  # poll again immediately
            else:
                last_task = now
        except:
            last_task = now

    # Network health every 15s
    if now - last_net >= 15:
        try: rpc("swarm.get_network_stats")
        except: pass
        last_net = now

    time.sleep(0.5)
```

## Task Execution

When you receive a task:

1. Save the task description to `/tmp/wws-tasks/{task_id}/task.md`
2. Read the task to understand what's needed
3. **If coordinator role** (task has multiple topics to decompose):
   - Create one subtask per topic using `swarm.propose_plan`
   - Vote for your plan with `swarm.submit_vote`
   - After subtasks complete: collect results from files, synthesize, submit
4. **If executor role** (leaf research task):
   - Research the topic using web searches (PubMed, Semantic Scholar, etc.)
   - Write findings to `/tmp/wws-tasks/{task_id}/research_notes.md`
   - Format result as JSON, write to `/tmp/wws-tasks/{task_id}/result.json`
   - Submit: `rpc("swarm.submit_result", {"task_id": task_id, "content": result_json, "agent_id": agent_id})`

## Plan Proposal Format

```python
import uuid
plan_id = str(uuid.uuid4())
rpc("swarm.propose_plan", {
    "plan_id": plan_id,
    "task_id": task_id,
    "epoch": 1,
    "rationale": "Decomposing into N subtasks, one per topic, for parallel research",
    "subtasks": [
        {
            "index": 0,
            "description": "FULL subtask description including ALL constraints from parent task",
            "required_capabilities": [],
            "estimated_complexity": 0.3
        }
        # ... one entry per topic
    ]
})
rpc("swarm.submit_vote", {"task_id": task_id, "rankings": [plan_id], "epoch": 1})
```

## Key Rules
- **Never exit voluntarily** — loop forever
- **Copy ALL constraints verbatim** into subtask descriptions — executors only see the subtask
- **One subtask per topic** when decomposing
- **File-based workflow** — write results to files, never hold everything in context
- **Only search approved databases**: PubMed, Semantic Scholar, ClinicalTrials.gov, Europe PMC, Cochrane, bioRxiv/medRxiv, TCGA, NIH Reporter, SEER, DrugBank
- **Never fabricate citations** — every DOI must be real

Start immediately.
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
