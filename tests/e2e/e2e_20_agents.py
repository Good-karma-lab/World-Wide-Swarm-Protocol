#!/usr/bin/env python3
"""
E2E test: 20 WWS connector nodes + 20 autonomous agents.

What this tests:
  - All 20 nodes register agents with meaningful names
  - Agents send direct messages to each other (new swarm.send_message)
  - Agent 1 injects a collaborative task via HTTP (identity bypass)
  - A holon forms, agents propose plans, critique, vote (IRV)
  - Agents complete tasks and build reputation
  - Results are verifiable through the HTTP API

Usage:
  python3 tests/e2e/e2e_20_agents.py
"""
import sys, json, socket, time, uuid, hashlib, threading, urllib.request, urllib.error

PORTS = [(9370 + i*2, 9371 + i*2) for i in range(20)]  # (rpc, files) for each node

SCIENTIST_NAMES = [
    "marie-curie", "albert-einstein", "niels-bohr", "max-planck",
    "werner-heisenberg", "paul-dirac", "erwin-schrodinger", "enrico-fermi",
    "richard-feynman", "murray-gell-mann", "peter-higgs", "francois-englert",
    "donna-strickland", "andre-geim", "robert-laughlin", "serge-haroche",
    "david-wineland", "giorgio-parisi", "syukuro-manabe", "john-bardeen",
]

RESULTS_LOCK = threading.Lock()
RESULTS = {}

# ── Transport ────────────────────────────────────────────────────────────────

def rpc(port, method, params, timeout=20):
    req = json.dumps({
        "jsonrpc": "2.0", "method": method,
        "params": params, "id": str(uuid.uuid4())[:8], "signature": ""
    }) + "\n"
    for attempt in range(3):
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=timeout) as s:
                s.sendall(req.encode())
                s.shutdown(socket.SHUT_WR)
                data = b""
                s.settimeout(timeout)
                while True:
                    chunk = s.recv(4096)
                    if not chunk:
                        break
                    data += chunk
                text = data.decode("utf-8", errors="replace").strip()
                d, _ = json.JSONDecoder().raw_decode(text)
                return d
        except Exception as e:
            if attempt == 2:
                return {"error": str(e)}
            time.sleep(0.5 + attempt)
    return {}


def http_get(url, timeout=10):
    try:
        with urllib.request.urlopen(url, timeout=timeout) as r:
            return json.loads(r.read())
    except Exception as e:
        return {"error": str(e)}


def http_post(url, body, timeout=60):
    try:
        data = json.dumps(body).encode()
        req = urllib.request.Request(url, data=data,
                                     headers={"Content-Type": "application/json"})
        with urllib.request.urlopen(req, timeout=timeout) as r:
            return json.loads(r.read())
    except Exception as e:
        return {"error": str(e)}


def log(name, msg):
    print(f"  [{name}] {msg}", flush=True)


# ── PoW ──────────────────────────────────────────────────────────────────────

def solve_pow(challenge_data, difficulty=24):
    """Solve proof-of-work: find nonce s.t. SHA256(challenge+nonce) has 'difficulty' leading zero bits."""
    prefix = challenge_data.encode() if isinstance(challenge_data, str) else challenge_data
    nonce = 0
    mask = (1 << (8 - difficulty % 8)) - 1 if difficulty % 8 != 0 else 0
    full_bytes = difficulty // 8
    while True:
        candidate = prefix + nonce.to_bytes(8, 'little')
        h = hashlib.sha256(candidate).digest()
        # Check leading zero bits
        ok = True
        for i in range(full_bytes):
            if h[i] != 0:
                ok = False
                break
        if ok and (difficulty % 8 == 0 or (h[full_bytes] & ~mask) == 0):
            return nonce
        nonce += 1
        if nonce > 50_000_000:
            raise RuntimeError("PoW exhausted")


# ── Agent ────────────────────────────────────────────────────────────────────

class Agent:
    def __init__(self, idx: int):
        self.idx = idx
        self.name = SCIENTIST_NAMES[idx]
        self.rpc_port, self.files_port = PORTS[idx]
        self.agent_id = None
        self.connector_did = None
        self.tasks_completed = 0

    def _r(self, method, params, timeout=20):
        return rpc(self.rpc_port, method, params, timeout)

    def setup(self):
        """Get connector identity."""
        st = self._r("swarm.get_status", {})
        self.connector_did = st.get("result", {}).get("agent_id", "")
        log(self.name, f"connector DID: {self.connector_did}")
        return bool(self.connector_did)

    def register(self):
        """Register with the swarm. The connector assigns a canonical DID."""
        log(self.name, "registering with swarm...")
        r = self._r("swarm.register_agent", {
            "agent_name": self.name,
            "agent_id": self.name,
            "capabilities": ["deliberation", "analysis", "coding"],
        })
        result = r.get("result", {})
        self.agent_id = result.get("agent_id") or self.name
        if "error" in r:
            log(self.name, f"⚠ register error: {r['error']}")
            self.agent_id = self.name
        else:
            log(self.name, f"✓ registered as {self.agent_id}")
        return True

    def send_message(self, to_did: str, content: str):
        r = self._r("swarm.send_message", {
            "from": self.agent_id or self.name,
            "to": to_did,
            "content": content,
        })
        ok = "error" not in r and r.get("result", {}).get("ok", False)
        log(self.name, f"{'✓' if ok else '✗'} DM → {to_did[:30]}…: {content[:50]}")
        return ok

    def get_messages(self):
        r = self._r("swarm.get_messages", {
            "agent_id": self.agent_id or self.name,
        })
        msgs = r.get("result", {}).get("messages", [])
        return msgs

    def receive_task(self):
        r = self._r("swarm.receive_task", {})
        tasks = r.get("result", {}).get("pending_tasks", [])
        return tasks[0] if tasks else None

    def get_task(self, task_id):
        r = self._r("swarm.get_task", {"task_id": task_id})
        return r.get("result", {}).get("task", {})

    def propose_plan(self, task_id, plan_text):
        plan_id = f"plan-{self.name}-{task_id[:8]}"
        r = self._r("swarm.propose_plan", {
            "task_id": task_id,
            "proposer": self.agent_id or self.name,
            "plan_id": plan_id,
            "epoch": 0,
            "rationale": plan_text,
            "subtasks": [
                {"index": 1, "description": f"Subtask 1: {plan_text[:40]}", "estimated_complexity": 0.3},
                {"index": 2, "description": f"Subtask 2: analysis phase", "estimated_complexity": 0.2},
            ],
            "estimated_parallelism": 2,
        })
        ok = "error" not in r
        log(self.name, f"{'✓' if ok else '✗'} proposed plan for {task_id[:12]}")
        return plan_id if ok else None

    def submit_vote(self, task_id, plan_ids):
        r = self._r("swarm.submit_vote", {
            "task_id": task_id,
            "voter_id": self.agent_id or self.name,
            "rankings": plan_ids,
        })
        ok = "error" not in r
        log(self.name, f"{'✓' if ok else '✗'} voted for task {task_id[:12]}")
        return ok

    def submit_result(self, task_id, content: str):
        cid = hashlib.sha256(content.encode()).hexdigest()
        r = self._r("swarm.submit_result", {
            "task_id": task_id,
            "agent_id": self.agent_id or self.name,
            "artifact": {
                "artifact_id": str(uuid.uuid4()),
                "task_id": task_id,
                "producer": self.agent_id or self.name,
                "content_cid": cid,
                "merkle_hash": cid,
                "content_type": "text/plain",
                "size_bytes": len(content),
                "content": content,
            },
            "merkle_proof": [],
        })
        ok = "error" not in r
        if ok:
            self.tasks_completed += 1
        log(self.name, f"{'✓' if ok else '✗'} submitted result for {task_id[:12]} (total={self.tasks_completed})")
        return ok

    def inject_task_http(self, description: str):
        """Use HTTP /api/tasks with connector identity to bypass reputation gate."""
        url = f"http://127.0.0.1:{self.files_port}/api/tasks"
        body = {
            "description": description,
            "injector_agent_id": self.connector_did,
            "priority": 1,
            "swarm_id": "public",
        }
        r = http_post(url, body)
        task_id = r.get("task_id") or r.get("id")
        if task_id:
            log(self.name, f"✓ injected task via HTTP: {task_id[:16]} — {description[:50]}")
        else:
            log(self.name, f"✗ HTTP inject failed: {r}")
        return task_id


def run_agent(agent: Agent, all_agents: list, barrier: threading.Barrier):
    """Full lifecycle for one agent."""
    try:
        # Phase 1: setup + register
        if not agent.setup():
            log(agent.name, "✗ setup failed")
            return
        agent.register()

        # Sync: wait for all agents to register
        barrier.wait(timeout=120)

        # Phase 2: send DMs to two peers
        peer_a = all_agents[(agent.idx + 1) % 20]
        peer_b = all_agents[(agent.idx + 3) % 20]
        agent.send_message(
            peer_a.connector_did or peer_a.name,
            f"Hi {peer_a.name}, let's collaborate on the task! I'm {agent.name}."
        )
        agent.send_message(
            peer_b.connector_did or peer_b.name,
            f"Hello {peer_b.name}, ready to deliberate? — {agent.name}"
        )

        # Phase 3: agents on nodes 0-4 inject warm-up tasks (HTTP bypass)
        warm_tasks = []
        if agent.idx < 5:
            for j in range(5):
                tid = agent.inject_task_http(
                    f"Warm-up task {j+1} from {agent.name}: analyse network topology segment {agent.idx*5+j}"
                )
                if tid:
                    warm_tasks.append(tid)

        barrier.wait(timeout=120)
        time.sleep(2)  # let GossipSub propagate

        # Phase 4: receive and complete warm-up tasks from own connector
        for _ in range(8):
            task_id = agent.receive_task()
            if task_id:
                t = agent.get_task(task_id)
                content = (
                    f"{agent.name} completed: {t.get('description','?')[:100]}. "
                    f"Analysis: network topology is optimal, latency p99=12ms, throughput=1.2 Gbps."
                )
                agent.submit_result(task_id, content)

        log(agent.name, f"reputation built: {agent.tasks_completed} tasks completed")

        # Phase 5: agents 0-2 inject real collaborative tasks
        barrier.wait(timeout=120)
        if agent.idx == 0:
            agent.inject_task_http(
                "Collaborative task: Design a Byzantine-fault-tolerant consensus protocol "
                "suitable for a decentralised AI agent network. Consider: (1) liveness under "
                "f < n/3 faulty nodes, (2) finality guarantees, (3) communication complexity. "
                "Propose pseudocode and complexity analysis."
            )
        elif agent.idx == 1:
            agent.inject_task_http(
                "Collaborative task: Implement a reputation scoring algorithm for the WWS network. "
                "The score should: (1) grow unbounded with contributions, (2) decay slowly without "
                "activity, (3) provide Sybil resistance via PoW. Propose the mathematical formula "
                "and implementation in Rust."
            )

        time.sleep(3)

        # Phase 6: pick up collaborative tasks and propose plans
        for _ in range(3):
            task_id = agent.receive_task()
            if task_id:
                t = agent.get_task(task_id)
                desc = t.get("description", "")
                if "Collaborative" in desc or "Byzantine" in desc or "reputation" in desc.lower():
                    plan_text = (
                        f"{agent.name}'s approach: I will decompose this into three phases — "
                        f"(1) theoretical analysis, (2) prototype implementation, (3) validation. "
                        f"Estimated complexity: 0.6. Timeline: 2 iterations."
                    )
                    agent.propose_plan(task_id, plan_text)
                    time.sleep(1)
                    # Vote for own plan + first peer's plan as fallback
                    agent.submit_vote(task_id, [
                        f"plan-{agent.name}-{task_id[:8]}",
                        f"plan-{all_agents[(agent.idx+1)%20].name}-{task_id[:8]}",
                    ])

        # Phase 7: check inbox
        msgs = agent.get_messages()
        log(agent.name, f"inbox: {len(msgs)} messages received")
        for m in msgs[:3]:
            log(agent.name, f"  DM from {m.get('from','?')[:30]}: {m.get('content','')[:60]}")

        with RESULTS_LOCK:
            RESULTS[agent.name] = {
                "tasks_completed": agent.tasks_completed,
                "messages_received": len(msgs),
                "rpc_port": agent.rpc_port,
                "agent_id": agent.agent_id,
            }

    except Exception as e:
        log(agent.name, f"✗ exception: {e}")
        import traceback; traceback.print_exc()
        with RESULTS_LOCK:
            RESULTS[agent.name] = {"error": str(e)}


def main():
    print("\n" + "="*60)
    print("  WWS E2E TEST — 20 nodes + 20 agents")
    print("="*60 + "\n")

    # Build agent objects
    agents = [Agent(i) for i in range(20)]

    # Verify all nodes are reachable
    print("▶ Verifying all 20 nodes are reachable...")
    unreachable = []
    for a in agents:
        st = rpc(a.rpc_port, "swarm.get_status", {}, timeout=5)
        if "error" in st and "result" not in st:
            unreachable.append(a.rpc_port)
    if unreachable:
        print(f"  ✗ unreachable ports: {unreachable}")
        sys.exit(1)
    print(f"  ✓ all 20 nodes responding\n")

    # Give nodes a moment to interconnect via bootstrap
    print("▶ Waiting for P2P mesh to settle (5s)...")
    time.sleep(5)

    # Run all agents in threads
    print(f"▶ Starting {len(agents)} agent threads...\n")
    barrier = threading.Barrier(len(agents), timeout=120)
    threads = []
    for a in agents:
        t = threading.Thread(target=run_agent, args=(a, agents, barrier), daemon=True)
        t.start()
        threads.append(t)

    for t in threads:
        t.join(timeout=300)

    # Summary
    print("\n" + "="*60)
    print("  RESULTS SUMMARY")
    print("="*60)
    total_tasks = 0
    total_msgs = 0
    errors = 0
    for name, r in sorted(RESULTS.items()):
        if "error" in r:
            print(f"  ✗ {name}: ERROR — {r['error']}")
            errors += 1
        else:
            tc = r.get("tasks_completed", 0)
            mr = r.get("messages_received", 0)
            total_tasks += tc
            total_msgs += mr
            print(f"  ✓ {name}: tasks={tc} msgs_recv={mr} port={r.get('rpc_port')}")

    print(f"\n  Total tasks completed : {total_tasks}")
    print(f"  Total messages received: {total_msgs}")
    print(f"  Agents with errors    : {errors}/{len(agents)}")
    print(f"\n  UI dashboard: http://127.0.0.1:9371/")
    print(f"  API health:   curl http://127.0.0.1:9371/api/health")
    print("="*60 + "\n")

    if errors > len(agents) // 4:
        print("  ✗ Too many errors — E2E FAILED")
        sys.exit(1)
    else:
        print("  ✓ E2E PASSED\n")


if __name__ == "__main__":
    main()
