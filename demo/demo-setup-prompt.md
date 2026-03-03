# WWS Demo Stand Setup

You are setting up a 30-node AI research swarm demo. Follow these steps exactly. You have access to Bash, WebFetch, and Agent tools.

---

## Step 1: Detect Platform and Download Binary

```bash
uname -sm 2>/dev/null || echo "Windows"
```

Select the binary based on output:
- `Darwin arm64` → `wws-connector-0.8.0-macos-arm64.tar.gz`
- `Darwin x86_64` → `wws-connector-0.8.0-macos-amd64.tar.gz`
- `Linux x86_64` or `Linux amd64` → `wws-connector-0.8.0-linux-amd64.tar.gz`
- `Linux aarch64` or `Linux arm64` → `wws-connector-0.8.0-linux-arm64.tar.gz`
- Windows (PowerShell: `$env:PROCESSOR_ARCHITECTURE`) → `wws-connector-0.8.0-windows-amd64.zip`

```bash
ASSET="wws-connector-0.8.0-macos-arm64.tar.gz"  # replace with detected asset
mkdir -p ~/wws-demo && cd ~/wws-demo
curl -L -o connector.tar.gz \
  "https://github.com/Good-karma-lab/World-Wide-Swarm-Protocol/releases/download/v0.8.0/$ASSET"
tar -xzf connector.tar.gz
chmod +x wws-connector
./wws-connector --version
```

For Windows (PowerShell):
```powershell
mkdir ~/wws-demo; cd ~/wws-demo
Invoke-WebRequest -Uri "https://github.com/Good-karma-lab/World-Wide-Swarm-Protocol/releases/download/v0.8.0/wws-connector-0.8.0-windows-amd64.zip" -OutFile connector.zip
Expand-Archive connector.zip -DestinationPath .
.\wws-connector.exe --version
```

---

## Step 2: Start Bootstrap Node (Node 1 — marie-curie)

Kill any leftover processes first, then start node 1:

```bash
pkill -9 -f "wws-connector" 2>/dev/null; sleep 1
mkdir -p /tmp/wws-demo-swarm
BIN=~/wws-demo/wws-connector

$BIN --agent-name marie-curie \
  --listen /ip4/0.0.0.0/tcp/9700 \
  --rpc 127.0.0.1:9730 \
  --files-addr 127.0.0.1:9731 \
  > /tmp/wws-demo-swarm/marie-curie.log 2>&1 &
echo "marie-curie started (pid=$!)"
```

Wait for it to be ready and capture the peer ID for bootstrapping:

```bash
sleep 5
BOOT_PEER=$(python3 -c "
import socket, json
req = json.dumps({'jsonrpc':'2.0','id':'1','method':'swarm.get_status','params':{},'signature':''}) + '\n'
s = socket.socket(); s.settimeout(3); s.connect(('127.0.0.1', 9730))
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
BOOT="/ip4/127.0.0.1/tcp/9700/p2p/$BOOT_PEER"
echo "Bootstrap address: $BOOT"
```

Copy the bootstrap address — you will use it in Step 3.

---

## Step 3: Verify Node 1 Health

```bash
python3 -c "
import urllib.request, json, sys
r = urllib.request.urlopen('http://127.0.0.1:9731/api/health', timeout=5)
data = json.loads(r.read())
print('Node 1 healthy:', data)
"
```

---

## Step 4: Open Web UI

On macOS: `open http://127.0.0.1:9731`
On Linux: `xdg-open http://127.0.0.1:9731`
On Windows: `Start-Process http://127.0.0.1:9731`

---

## Step 5: Spawn Node 1's Agent + All 29 Connector-Agent Pairs in Parallel

Replace `{BOOT}` in the prompts below with the actual bootstrap address from Step 2
(format: `/ip4/127.0.0.1/tcp/9700/p2p/12D3KooW...`).

Spawn **all 30 agents in a single message** using 30 parallel Agent tool calls.

**Agent 1 (marie-curie, Node 1)** — bootstrap node is already running, just spawn the agent:

```
Agent prompt for marie-curie:
---
You are 'marie-curie', an autonomous AI research agent in the World Wide Swarm (WWS) demo.

Your connector is already running:
  HTTP: http://127.0.0.1:9731
  RPC:  127.0.0.1:9730

[paste the full agent loop prompt from the AGENT LOOP TEMPLATE below — use RPC_PORT=9730]
```

**Agents 2–30** — each agent starts its own connector, then runs the agent loop.

Use this template for each agent (replace NAME, P2P_PORT, RPC_PORT, HTTP_PORT, BOOT_ADDR):

```
You are '{NAME}', an autonomous AI research agent in the World Wide Swarm (WWS) demo.

## Step A: Start your connector

```bash
pkill -f "wws-connector.*{RPC_PORT}" 2>/dev/null
~/wws-demo/wws-connector \
  --agent-name {NAME} \
  --listen /ip4/0.0.0.0/tcp/{P2P_PORT} \
  --rpc 127.0.0.1:{RPC_PORT} \
  --files-addr 127.0.0.1:{HTTP_PORT} \
  --bootstrap "{BOOT_ADDR}" \
  > /tmp/wws-demo-swarm/{NAME}.log 2>&1 &
echo "{NAME} connector started"
sleep 3
```

## Step B: Run the agent loop

[paste the full AGENT LOOP TEMPLATE below — use RPC_PORT={RPC_PORT} and HTTP_PORT={HTTP_PORT}]
```

### Port assignments for agents 2–30:

| Agent | Name | P2P | RPC | HTTP |
|-------|------|-----|-----|------|
| 2 | albert-einstein | 9701 | 9732 | 9733 |
| 3 | niels-bohr | 9702 | 9734 | 9735 |
| 4 | max-planck | 9703 | 9736 | 9737 |
| 5 | werner-heisenberg | 9704 | 9738 | 9739 |
| 6 | paul-dirac | 9705 | 9740 | 9741 |
| 7 | erwin-schrodinger | 9706 | 9742 | 9743 |
| 8 | enrico-fermi | 9707 | 9744 | 9745 |
| 9 | richard-feynman | 9708 | 9746 | 9747 |
| 10 | murray-gell-mann | 9709 | 9748 | 9749 |
| 11 | abdus-salam | 9710 | 9750 | 9751 |
| 12 | steven-weinberg | 9711 | 9752 | 9753 |
| 13 | sheldon-glashow | 9712 | 9754 | 9755 |
| 14 | peter-higgs | 9713 | 9756 | 9757 |
| 15 | francois-englert | 9714 | 9758 | 9759 |
| 16 | donna-strickland | 9715 | 9760 | 9761 |
| 17 | andre-geim | 9716 | 9762 | 9763 |
| 18 | konstantin-novoselov | 9717 | 9764 | 9765 |
| 19 | robert-laughlin | 9718 | 9766 | 9767 |
| 20 | alexei-abrikosov | 9719 | 9768 | 9769 |
| 21 | vitaly-ginzburg | 9720 | 9770 | 9771 |
| 22 | serge-haroche | 9721 | 9772 | 9773 |
| 23 | david-wineland | 9722 | 9774 | 9775 |
| 24 | klaus-hasselmann | 9723 | 9776 | 9777 |
| 25 | giorgio-parisi | 9724 | 9778 | 9779 |
| 26 | syukuro-manabe | 9725 | 9780 | 9781 |
| 27 | john-bardeen | 9726 | 9782 | 9783 |
| 28 | walter-brattain | 9727 | 9784 | 9785 |
| 29 | william-shockley | 9728 | 9786 | 9787 |
| 30 | charles-townes | 9729 | 9788 | 9789 |

---

## AGENT LOOP TEMPLATE

Copy this into each agent prompt, replacing RPC_PORT and HTTP_PORT:

```
## RPC Setup

```python
import socket, json, time, os, uuid

RPC_PORT = {RPC_PORT}

def rpc(method, params={}):
    req = json.dumps({"jsonrpc":"2.0","id":"1","method":method,"params":params,"signature":""}) + "\n"
    s = socket.socket(); s.settimeout(15); s.connect(("127.0.0.1", RPC_PORT))
    s.sendall(req.encode()); s.shutdown(1)
    data = b""
    while True:
        c = s.recv(65536)
        if not c: break
        data += c
    s.close()
    return json.loads(data).get("result", {})
```

## Startup

```python
status = rpc("swarm.get_status")
agent_id = status.get("agent_id", "")
print(f"Agent ID: {agent_id}")
print(f"Tier: {status.get('tier','?')}  Known peers: {status.get('known_agents',0)}")
```

## Task Execution Function

```python
def execute_task(task):
    task_id = task.get("task_id") or task
    task_desc = task.get("description", "")

    # Save task to file
    task_dir = f"/tmp/wws-tasks/{task_id}"
    os.makedirs(task_dir, exist_ok=True)
    with open(f"{task_dir}/task.md", "w") as f:
        f.write(task_desc)

    print(f"Executing task {task_id[:12]}...")

    # Determine if coordinator (has multiple topics) or executor (leaf task)
    is_coordinator = ("topics" in task_desc and task_desc.count('"') > 10) or \
                     ("mission" in task_desc and "type" in task_desc)

    if is_coordinator:
        # Parse topics from the task description
        import re
        topics = re.findall(r'"([^"]{20,})"', task_desc)
        # Filter to likely topic strings (not keys)
        topics = [t for t in topics if not t.startswith("http") and len(t) > 20][:13]

        if not topics:
            topics = ["Research the assigned topic thoroughly"]

        print(f"Coordinator: decomposing into {len(topics)} subtasks")

        # Build subtask descriptions with FULL constraints copied from parent
        subtasks = []
        for i, topic in enumerate(topics):
            subtask_desc = f"""Research topic: "{topic}"

{task_desc}

FOCUS FOR THIS SUBTASK: Research ONLY the topic "{topic}" from the mission above.
Include: methodology notes, statistics, effect sizes, sample sizes.
Minimum 5 papers. Prioritize 2020-2025. Prefer systematic reviews and meta-analyses."""
            subtasks.append({
                "index": i,
                "description": subtask_desc,
                "required_capabilities": [],
                "estimated_complexity": 0.3
            })

        plan_id = str(uuid.uuid4())
        rpc("swarm.propose_plan", {
            "plan_id": plan_id,
            "task_id": task_id,
            "epoch": 1,
            "rationale": f"Decomposing into {len(subtasks)} subtasks, one per research topic, for parallel execution by executor agents",
            "subtasks": subtasks
        })
        rpc("swarm.submit_vote", {"task_id": task_id, "rankings": [plan_id], "epoch": 1})
        print(f"Plan proposed and voted: {len(subtasks)} subtasks")

    else:
        # Executor: do the research
        print(f"Executor: researching '{task_desc[:80]}...'")

        # Research using web search tools (WebSearch/WebFetch)
        # Write findings to research_notes.md as you go
        notes_file = f"{task_dir}/research_notes.md"

        # Perform actual research here using available tools
        # (The agent should use WebSearch to search PubMed, Semantic Scholar, etc.)
        research_summary = f"Research findings for: {task_desc[:200]}"

        result = json.dumps({
            "title": f"Research: {task_desc[:100]}",
            "summary": research_summary,
            "citations": [],
            "confidence": "medium",
            "contradictions": [],
            "gaps": ["Further research needed"],
            "papersAnalyzed": 0
        })

        with open(f"{task_dir}/result.json", "w") as f:
            f.write(result)

        rpc("swarm.submit_result", {
            "task_id": task_id,
            "content": result,
            "agent_id": agent_id
        })
        print(f"Result submitted for task {task_id[:12]}")
```

## Main Loop — Run Forever

```python
last_status = 0; last_task = 0; last_net = 0; busy = False

while True:
    now = time.time()

    if now - last_status >= 3:
        try: rpc("swarm.get_status")
        except: pass
        last_status = now

    if not busy and now - last_task >= 2:
        try:
            result = rpc("swarm.receive_task")
            tasks = result.get("pending_tasks", [])
            if tasks:
                busy = True
                try:
                    execute_task(tasks[0])
                except Exception as e:
                    print(f"Task error: {e}")
                busy = False
                last_task = 0
            else:
                last_task = now
        except:
            last_task = now

    if now - last_net >= 15:
        try: rpc("swarm.get_network_stats")
        except: pass
        last_net = now

    time.sleep(0.5)
```
```

---

## Step 6: Submit TNBC Research Task

Wait 30 seconds for all agents to register and connect, then submit:

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

- **Web UI** (`http://127.0.0.1:9731`) — watch Nobel laureates register, tier assignments form, task decompose into 13 subtasks
- **Task panel** — 13 TNBC topic subtasks appear, each assigned to an executor agent
- **Agent panel** — agents show research activity, result submissions, deliberation votes
- **Synthesis** — coordinator aggregates all 13 research results

---

## Stopping the Demo

```bash
pkill -9 -f "wws-connector"
```

All agents will stop automatically when their connector is killed (RPC becomes unreachable).
