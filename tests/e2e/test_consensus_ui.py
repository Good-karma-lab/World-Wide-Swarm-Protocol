#!/usr/bin/env python3
"""
E2E test: two real wws-connector instances, full consensus lifecycle,
UI assertions via HTTP. No fallbacks — asserts hard on every step.

Run: python3 tests/e2e/test_consensus_ui.py /path/to/wws-connector
"""
import json
import socket
import subprocess
import sys
import time
import os
import signal
import urllib.request

BINARY = sys.argv[1] if len(sys.argv) > 1 else "wws-connector"

A_RPC   = ("127.0.0.1", 9370)
A_HTTP  = "http://127.0.0.1:9371"
B_RPC   = ("127.0.0.1", 9380)
B_HTTP  = "http://127.0.0.1:9381"

procs = []

# ── RPC helpers ───────────────────────────────────────────────────────────────

def rpc(addr, method, params, timeout=30):
    msg = json.dumps({
        "jsonrpc": "2.0", "method": method, "id": "1",
        "params": params,
        "signature": "test-driver",
    }).encode()
    with socket.create_connection(addr, timeout=timeout) as s:
        s.sendall(msg + b"\n")   # server reads line-by-line
        s.settimeout(timeout)
        buf = b""
        while True:
            chunk = s.recv(65536)
            if not chunk:
                break
            buf += chunk
            try:
                resp = json.loads(buf)
                break
            except json.JSONDecodeError:
                continue
    assert "error" not in resp, f"{method} error: {resp['error']}"
    return resp["result"]

def http_get(url):
    with urllib.request.urlopen(url, timeout=8) as r:
        return json.loads(r.read())

# ── Startup helpers ───────────────────────────────────────────────────────────

def start_connector(rpc_addr, files_addr, name):
    p = subprocess.Popen(
        [BINARY, "--rpc", rpc_addr, "--files-addr", files_addr, "--agent-name", name],
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
    )
    procs.append(p)
    return p

def wait_healthy(http_base, label, retries=20):
    for _ in range(retries):
        try:
            d = http_get(f"{http_base}/api/health")
            if d.get("ok"):
                print(f"  {label} healthy")
                return
        except Exception:
            pass
        time.sleep(0.5)
    raise AssertionError(f"{label} did not become healthy in time")

def get_libp2p_port(pid, retries=20):
    """Find the libp2p listening port via lsof.
    Uses -a (AND logic) so only this process's TCP LISTEN ports are returned.
    Excludes the RPC port (9370) and HTTP port (9371).
    """
    excluded = {A_RPC[1], int(A_HTTP.split(":")[-1])}
    for _ in range(retries):
        out = subprocess.check_output(
            ["lsof", "-a", "-p", str(pid), "-i", "TCP", "-n"],
            stderr=subprocess.DEVNULL, text=True,
        )
        for line in out.splitlines():
            if "LISTEN" not in line:
                continue
            # field[-2] is the address:port field
            parts = line.split()
            addr_field = parts[-2] if len(parts) > 1 else ""
            port_str = addr_field.split(":")[-1]
            if port_str.isdigit():
                port = int(port_str)
                if port not in excluded:
                    return port
        time.sleep(0.5)
    raise AssertionError(f"Could not find libp2p port for pid {pid}")

def cleanup():
    for p in procs:
        try:
            p.terminate()
            p.wait(timeout=3)
        except Exception:
            p.kill()

# ── Test ──────────────────────────────────────────────────────────────────────

def run():
    print("=== E2E: Two-connector consensus lifecycle ===\n")

    # 1. Start both connectors
    print("[1] Starting connectors…")
    pA = start_connector("127.0.0.1:9370", "127.0.0.1:9371", "AlphaAgent")
    pB = start_connector("127.0.0.1:9380", "127.0.0.1:9381", "BetaAgent")
    wait_healthy(A_HTTP, "AlphaAgent (A)")
    wait_healthy(B_HTTP, "BetaAgent (B)")

    # 2. Connect A ↔ B via P2P
    print("[2] Connecting P2P…")
    p2p_port = get_libp2p_port(pA.pid)
    peer_id_a = http_get(f"{A_HTTP}/api/agents")["agents"][0]["agent_id"].replace("did:swarm:", "")
    multiaddr = f"/ip4/127.0.0.1/tcp/{p2p_port}/p2p/{peer_id_a}"
    print(f"  A multiaddr: {multiaddr}")
    rpc(B_RPC, "swarm.connect", {"addr": multiaddr})
    # wait for peer_link edge to appear (libp2p handshake takes a few seconds)
    for _ in range(30):
        topo = http_get(f"{A_HTTP}/api/topology")
        if any(e.get("kind") == "peer_link" for e in topo.get("edges", [])):
            break
        time.sleep(0.5)
    topo = http_get(f"{A_HTTP}/api/topology")
    assert len(topo["nodes"]) >= 2, f"Expected ≥2 topology nodes, got {topo['nodes']}"
    assert len(topo["edges"]) >= 1, f"Expected ≥1 peer_link edge, got {topo['edges']}"
    print(f"  Topology: {len(topo['nodes'])} nodes, {len(topo['edges'])} edges")

    # Verify BetaAgent name propagated to A
    for _ in range(20):
        agents_a = http_get(f"{A_HTTP}/api/agents")["agents"]
        names = [a["name"] for a in agents_a]
        if "BetaAgent" in names:
            break
        time.sleep(0.5)
    agents_a = http_get(f"{A_HTTP}/api/agents")["agents"]
    names_a = [a["name"] for a in agents_a]
    assert "AlphaAgent" in names_a, f"AlphaAgent missing: {names_a}"
    assert "BetaAgent"  in names_a, f"BetaAgent missing from A's view: {names_a}"
    print(f"  A sees agents: {names_a}")

    # 3. Inject task
    print("[3] Injecting task…")
    task_result = rpc(A_RPC, "swarm.inject_task", {
        "description": "Build a distributed key-value store with consistent hashing, replication factor 3, and gossip-based failure detection",
        "tier": 1,
        "expected_proposers": 2,
    })
    task_id = task_result["task_id"]
    print(f"  task_id: {task_id}")

    # 4. AlphaAgent proposes plan
    print("[4] AlphaAgent proposes plan…")
    import uuid
    plan_id_a = str(uuid.uuid4())[:8]
    # Get current epoch before proposing so votes can use the same epoch
    net_stats_a = rpc(A_RPC, "swarm.get_network_stats", {})
    vote_epoch = net_stats_a.get("epoch", 0)
    print(f"  Current epoch: {vote_epoch}")

    rpc(A_RPC, "swarm.propose_plan", {
        "plan_id": plan_id_a,
        "task_id": task_id,
        "epoch": vote_epoch,
        "rationale": "Shard data by consistent hash ring, replicate to 3 nodes, gossip heartbeats every 500ms",
        "subtasks": [
            {"index": 0, "description": "Implement consistent hash ring with virtual nodes", "required_capabilities": [], "estimated_complexity": 0.5},
            {"index": 1, "description": "Build replication protocol with quorum writes", "required_capabilities": [], "estimated_complexity": 0.6},
            {"index": 2, "description": "Implement gossip failure detector with suspicion mechanism", "required_capabilities": [], "estimated_complexity": 0.4},
            {"index": 3, "description": "Add client SDK with transparent retry and routing", "required_capabilities": [], "estimated_complexity": 0.3},
        ],
    })
    print(f"  plan_id_a: {plan_id_a}")

    # 5. BetaAgent proposes plan from B
    print("[5] BetaAgent proposes plan…")
    plan_id_b = str(uuid.uuid4())[:8]
    rpc(B_RPC, "swarm.propose_plan", {
        "plan_id": plan_id_b,
        "task_id": task_id,
        "epoch": vote_epoch,
        "rationale": "Use Raft consensus per shard, strong consistency guarantees, linearisable reads",
        "subtasks": [
            {"index": 0, "description": "Implement Raft leader election per shard", "required_capabilities": [], "estimated_complexity": 0.7},
            {"index": 1, "description": "Build log replication with back-pressure", "required_capabilities": [], "estimated_complexity": 0.6},
            {"index": 2, "description": "Add read-index for linearisable reads", "required_capabilities": [], "estimated_complexity": 0.4},
        ],
    })
    print(f"  plan_id_b: {plan_id_b}")

    # Wait for both plans to appear in RFP
    for _ in range(20):
        rfp = http_get(f"{A_HTTP}/api/voting/{task_id}")
        plans = rfp.get("rfp", [{}])[0].get("plans", []) if rfp.get("rfp") else []
        if len(plans) >= 1:
            break
        time.sleep(0.5)

    # 6. Both agents submit votes using the same epoch as the plans
    print("[6] Submitting votes…")
    rpc(A_RPC, "swarm.submit_vote", {
        "task_id": task_id,
        "epoch": vote_epoch,
        "rankings": [plan_id_a, plan_id_b],
    })
    rpc(B_RPC, "swarm.submit_vote", {
        "task_id": task_id,
        "epoch": vote_epoch,
        "rankings": [plan_id_b, plan_id_a],
    })
    print("  Both votes submitted")

    # 7. Both agents submit critiques
    print("[7] Submitting critiques…")
    critique_a = {
        "task_id": task_id,
        "content": "AlphaAgent critique: The Raft approach offers strong consistency but has higher latency during leader elections. The gossip approach handles partitions more gracefully for a KV workload.",
        "plan_scores": {
            plan_id_a: {"feasibility": 0.85, "completeness": 0.90, "parallelism": 0.75, "risk": 0.20},
            plan_id_b: {"feasibility": 0.70, "completeness": 0.80, "parallelism": 0.50, "risk": 0.40},
        },
    }
    rpc(A_RPC, "swarm.submit_critique", critique_a)

    critique_b = {
        "task_id": task_id,
        "content": "BetaAgent critique: Consistent hashing with gossip is simpler but risks stale reads. Raft gives us linearisability out of the box, which is essential for a KV store used in distributed transactions.",
        "plan_scores": {
            plan_id_a: {"feasibility": 0.75, "completeness": 0.70, "parallelism": 0.80, "risk": 0.30},
            plan_id_b: {"feasibility": 0.88, "completeness": 0.92, "parallelism": 0.55, "risk": 0.25},
        },
    }
    rpc(B_RPC, "swarm.submit_critique", critique_b)
    print("  Both critiques submitted")

    # Wait for data to propagate + voting_check_interval (5s) to run
    print("[8] Waiting for voting quorum check (≤10s)…")
    for _ in range(20):
        irv = http_get(f"{A_HTTP}/api/tasks/{task_id}/irv-rounds")
        if irv.get("irv_rounds"):
            break
        time.sleep(0.5)
    print(f"  IRV rounds: {len(irv.get('irv_rounds', []))}")

    # ── Assertions ────────────────────────────────────────────────────────────
    print("\n=== Asserting all panels have real data ===\n")

    # Topology: ≥2 nodes, ≥1 edge
    topo = http_get(f"{A_HTTP}/api/topology")
    assert len(topo["nodes"]) >= 2, f"FAIL topology nodes: {topo['nodes']}"
    assert len(topo["edges"]) >= 1, f"FAIL topology edges: {topo['edges']}"
    peer_links = [e for e in topo["edges"] if e.get("kind") == "peer_link"]
    assert len(peer_links) >= 1, f"FAIL no peer_link edges: {topo['edges']}"
    print(f"  PASS  Topology: {len(topo['nodes'])} nodes, {len(peer_links)} peer_link edge(s)")

    # Agents: both names visible
    agents_a = http_get(f"{A_HTTP}/api/agents")["agents"]
    names_a = [a["name"] for a in agents_a]
    assert "AlphaAgent" in names_a, f"FAIL AlphaAgent missing: {names_a}"
    assert "BetaAgent"  in names_a, f"FAIL BetaAgent missing: {names_a}"
    alpha = next(a for a in agents_a if a["name"] == "AlphaAgent")
    assert alpha["plans_proposed_count"] >= 1, f"FAIL AlphaAgent plans_proposed=0"
    assert alpha["votes_cast_count"] >= 1, f"FAIL AlphaAgent votes_cast=0"
    print(f"  PASS  Agents: {names_a}, AlphaAgent plans={alpha['plans_proposed_count']} votes={alpha['votes_cast_count']}")

    # Task timeline: multiple events
    trace = http_get(f"{A_HTTP}/api/tasks/{task_id}/timeline")
    assert len(trace["timeline"]) >= 2, f"FAIL timeline has <2 events: {trace['timeline']}"
    print(f"  PASS  Timeline: {len(trace['timeline'])} events")

    # Voting: RFP with plans
    rfp_resp = http_get(f"{A_HTTP}/api/voting/{task_id}")
    rfp_list = rfp_resp.get("rfp", [])
    assert len(rfp_list) >= 1, f"FAIL no RFP for task"
    rfp = rfp_list[0]
    assert len(rfp.get("plans", [])) >= 1, f"FAIL no plans in RFP: {rfp}"
    print(f"  PASS  Voting: phase={rfp['phase']}, plans={len(rfp['plans'])}")

    # Ballots
    ballots_resp = http_get(f"{A_HTTP}/api/tasks/{task_id}/ballots")
    ballots = ballots_resp.get("ballots", [])
    assert len(ballots) >= 1, f"FAIL no ballots recorded: {ballots_resp}"
    assert any(b.get("critic_scores") for b in ballots), f"FAIL no critic_scores in ballots"
    print(f"  PASS  Ballots: {len(ballots)} ballot(s), with critic_scores")

    # IRV rounds
    irv_resp = http_get(f"{A_HTTP}/api/tasks/{task_id}/irv-rounds")
    irv_rounds = irv_resp.get("irv_rounds", [])
    assert len(irv_rounds) >= 1, f"FAIL no IRV rounds: {irv_resp}"
    print(f"  PASS  IRV rounds: {len(irv_rounds)} round(s)")

    # Deliberation messages
    delib = http_get(f"{A_HTTP}/api/tasks/{task_id}/deliberation")
    msgs = delib.get("messages", [])
    assert len(msgs) >= 1, f"FAIL no deliberation messages"
    critique_msgs = [m for m in msgs if m.get("message_type") == "CritiqueFeedback"]
    assert len(critique_msgs) >= 1, f"FAIL no CritiqueFeedback messages: {[m['message_type'] for m in msgs]}"
    assert any(m.get("critic_scores") for m in critique_msgs), f"FAIL critique messages missing critic_scores"
    print(f"  PASS  Deliberation: {len(msgs)} message(s), {len(critique_msgs)} critique(s)")

    # P2P messages
    msgs_resp = http_get(f"{A_HTTP}/api/messages")
    p2p_msgs = msgs_resp if isinstance(msgs_resp, list) else msgs_resp.get("messages", [])
    assert len(p2p_msgs) >= 1, f"FAIL no P2P messages recorded"
    print(f"  PASS  P2P messages: {len(p2p_msgs)} message(s)")

    # Audit log
    audit_resp = http_get(f"{A_HTTP}/api/audit")
    events = audit_resp.get("events", [])
    assert len(events) >= 2, f"FAIL audit has <2 events: {events}"
    event_types = [e.get("event_type", e.get("stage", "")) for e in events]
    print(f"  PASS  Audit: {len(events)} event(s): {event_types[:4]}")

    print(f"\n=== ALL ASSERTIONS PASSED ===")
    print(f"\nConnectors still running for manual UI inspection:")
    print(f"  AlphaAgent UI → {A_HTTP}")
    print(f"  Task ID       → {task_id}")

    return task_id

if __name__ == "__main__":
    try:
        task_id = run()
        # Keep connectors alive for UI verification
        print("\nPress Ctrl+C to stop connectors.")
        signal.pause()
    except KeyboardInterrupt:
        pass
    except Exception as e:
        print(f"\nFAIL: {e}")
        cleanup()
        sys.exit(1)
    finally:
        cleanup()
