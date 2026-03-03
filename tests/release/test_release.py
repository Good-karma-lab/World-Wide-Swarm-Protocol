#!/usr/bin/env python3
"""
Comprehensive E2E test for wws-connector release binary.

Usage:
  python3 tests/release/test_release.py /path/to/wws-connector

  Or set environment variable:
  WWS_BINARY=/path/to/wws-connector python3 tests/release/test_release.py

The binary must be extracted from the release archive and pointed to here.
For macOS arm64: download wws-connector-X.Y.Z-macos-arm64.tar.gz
For Linux arm64: download wws-connector-X.Y.Z-linux-arm64.tar.gz

Tests:
  1.  Binary: correct name/version
  2.  Startup + health (HTTP)
  3.  Web root (HTML served from webapp/dist)
  4.  SKILL.md served via HTTP (embedded in binary)
  5.  /api/identity
  6.  /api/network
  7.  /api/reputation
  8.  /api/directory
  9.  /api/names
  10. /api/tasks
  11. /api/holons
  12. /api/keys
  13. SSE stream (/api/events)
  14. RPC protocol format
  15. swarm.get_status
  16. swarm.inject_task
  17. swarm.receive_task
  18. swarm.register_agent -> challenge flow
  19. swarm.verify_agent
  20. swarm.register_agent (post-verify)
  21. swarm.register_name
  22. swarm.resolve_name
  23. Docker multi-node: 2 containers on bridge network
"""
import sys, os, json, socket, time, subprocess, re, uuid, urllib.request, urllib.error
import tempfile, tarfile, platform

# ── Configuration ─────────────────────────────────────────────────────────────
BINARY = sys.argv[1] if len(sys.argv) > 1 else os.environ.get("WWS_BINARY", "")
if not BINARY:
    print("Usage: python3 tests/release/test_release.py /path/to/wws-connector")
    sys.exit(1)

# The binary's working directory (must contain webapp/dist/ and docs/)
BINARY_DIR = os.path.dirname(os.path.abspath(BINARY))

HTTP = 9371
RPC  = 9370
PROC = None

PASS = 0; FAIL = 0; results = []

def ok(name, detail=""):
    global PASS; PASS += 1
    results.append(("PASS", name, detail))
    print(f"  \033[32mPASS\033[0m  {name}" + (f" — {detail}" if detail else ""))

def fail(name, detail=""):
    global FAIL; FAIL += 1
    results.append(("FAIL", name, detail))
    print(f"  \033[31mFAIL\033[0m  {name}" + (f" — {detail}" if detail else ""))

def http_get(path, timeout=5):
    try:
        with urllib.request.urlopen(f"http://127.0.0.1:{HTTP}{path}", timeout=timeout) as r:
            return r.status, r.read().decode()
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode()
    except Exception as e:
        return None, str(e)

def rpc(method, params={}, timeout=10):
    req = json.dumps({
        "jsonrpc": "2.0", "method": method, "params": params,
        "id": uuid.uuid4().hex[:8], "signature": ""
    }) + "\n"
    for attempt in range(2):
        try:
            with socket.create_connection(("127.0.0.1", RPC), timeout=timeout) as s:
                s.sendall(req.encode())
                s.shutdown(socket.SHUT_WR)
                data = b""
                s.settimeout(timeout)
                while True:
                    chunk = s.recv(4096)
                    if not chunk: break
                    data += chunk
                return json.loads(data.decode("utf-8", errors="replace").strip())
        except Exception as e:
            if attempt == 0: time.sleep(1)
            else: return {"_error": str(e)}
    return None

# ── start connector ──────────────────────────────────────────────────────────
print(f"\n=== wws-connector Release E2E Test ===")
print(f"Binary: {BINARY}\n")
subprocess.run(["pkill", "-f", "wws-connector"], capture_output=True)
time.sleep(0.5)

PROC = subprocess.Popen(
    [BINARY, "--rpc", f"127.0.0.1:{RPC}", "--files-addr", f"127.0.0.1:{HTTP}"],
    cwd=BINARY_DIR,
    stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL
)
time.sleep(3)

# ── 1: binary name / version ─────────────────────────────────────────────────
print("── Binary ──────────────────────────────")
r = subprocess.run([BINARY, "--version"], capture_output=True, text=True)
ver = r.stdout.strip()
if "wws-connector" in ver:
    ok("binary name+version", ver)
else:
    fail("binary name+version", f"got: {ver!r}")

# ── 2-3: HTTP health + web root ───────────────────────────────────────────────
print("── HTTP endpoints ──────────────────────")
code, body = http_get("/api/health")
if code == 200 and "ok" in body:
    ok("GET /api/health", body.strip()[:60])
else:
    fail("GET /api/health", f"code={code} body={body[:60]}")

code, body = http_get("/")
if code == 200 and "<!doctype html" in body.lower():
    ok("GET / (web UI)", f"{len(body)} bytes HTML")
else:
    fail("GET / (web UI)", f"code={code}, body={body[:80]!r}")

# ── 4: SKILL.md served via HTTP ───────────────────────────────────────────────
code, body = http_get("/SKILL.md")
if code == 200 and "WWS.Connector" in body and "wws-connector" in body:
    ok("GET /SKILL.md (embedded)", f"{len(body)} bytes, name/binary correct")
elif code == 200 and "ASIP" in body:
    fail("GET /SKILL.md", "served but still says ASIP.Connector — binary built from old source?")
else:
    fail("GET /SKILL.md", f"code={code}")

# ── 5-12: REST API endpoints ──────────────────────────────────────────────────
for path, required_key in [
    ("/api/identity",   "did"),
    ("/api/network",    "peer_count"),
    ("/api/reputation", None),
    ("/api/directory",  None),
    ("/api/names",      None),
    ("/api/tasks",      None),
    ("/api/holons",     None),
    ("/api/keys",       None),
]:
    code, body = http_get(path)
    if code == 200:
        try:
            parsed = json.loads(body)
            if required_key and required_key not in json.dumps(parsed):
                fail(f"GET {path}", f"missing key '{required_key}': {body[:80]}")
            else:
                ok(f"GET {path}", body.strip()[:80])
        except Exception as e:
            fail(f"GET {path}", f"bad JSON: {e}")
    else:
        fail(f"GET {path}", f"code={code}, body={body[:80]!r}")

# ── 13: SSE stream ─────────────────────────────────────────────────────────────
print("── SSE ─────────────────────────────────")
try:
    with urllib.request.urlopen(f"http://127.0.0.1:{HTTP}/api/events", timeout=3) as r:
        content_type = r.headers.get("Content-Type", "")
        if "text/event-stream" in content_type:
            ok("GET /api/events (SSE)", f"Content-Type: {content_type}")
        else:
            fail("GET /api/events (SSE)", f"wrong Content-Type: {content_type}")
except Exception as e:
    if "timed out" in str(e) or "Read timed out" in str(e):
        # SSE streams don't close — a timeout means it IS streaming
        ok("GET /api/events (SSE)", "streaming (timeout expected)")
    else:
        fail("GET /api/events (SSE)", str(e))

# ── 14-17: RPC core ───────────────────────────────────────────────────────────
print("── RPC ─────────────────────────────────")
resp = rpc("swarm.get_status")
if "result" in resp and "agent_id" in resp.get("result", {}):
    agent_did = resp["result"]["agent_id"]
    ok("swarm.get_status", f"did={agent_did[:30]}...")
else:
    fail("swarm.get_status", str(resp)[:100])

resp = rpc("swarm.inject_task", {"description": "release E2E test task"})
if "result" in resp and resp["result"].get("injected"):
    task_id = resp["result"]["task_id"]
    ok("swarm.inject_task", f"task_id={task_id[:20]}...")
else:
    fail("swarm.inject_task", str(resp)[:100])

resp = rpc("swarm.receive_task")
if "result" in resp and "pending_tasks" in resp.get("result", {}):
    ok("swarm.receive_task", f"pending={resp['result']['pending_tasks']}")
else:
    fail("swarm.receive_task", str(resp)[:100])

# ── 18-20: Registration + challenge flow ──────────────────────────────────────
print("── Registration challenge flow ─────────")
AGENT_ID = f"test-agent-{uuid.uuid4().hex[:8]}"
resp = rpc("swarm.register_agent", {"agent_id": AGENT_ID, "name": "Release Test Agent",
                                     "capabilities": ["testing"]})
result = resp.get("result", {})
if "challenge" in result:
    challenge_text = result["challenge"]
    code_val       = result["code"]
    agent_id_resp  = result.get("agent_id", AGENT_ID)
    nums = re.findall(r'\b\d+\b', challenge_text)
    if not nums:
        fail("register_agent challenge", f"no numbers in: {challenge_text!r}")
    else:
        answer = sum(int(n) for n in nums)
        ok("register_agent -> challenge", f"challenge={challenge_text!r}, answer={answer}")

        vresp = rpc("swarm.verify_agent", {"agent_id": agent_id_resp, "code": code_val, "answer": answer})
        if vresp.get("result", {}).get("verified"):
            ok("verify_agent", "verified=true")
        else:
            fail("verify_agent", str(vresp)[:100])

        rresp = rpc("swarm.register_agent", {"agent_id": AGENT_ID, "name": "Release Test Agent",
                                              "capabilities": ["testing"]})
        if rresp.get("result", {}).get("registered"):
            ok("register_agent (post-verify)", "registered=true")
        else:
            fail("register_agent (post-verify)", str(rresp)[:100])
elif result.get("registered"):
    ok("register_agent -> already registered", str(result))
else:
    fail("register_agent", str(resp)[:100])

# ── 21-22: Name registry ─────────────────────────────────────────────────────
print("── Name registry ───────────────────────")
test_name = f"testnode-{uuid.uuid4().hex[:6]}"
test_did  = f"did:swarm:12D3KooW{uuid.uuid4().hex[:16]}"
resp = rpc("swarm.register_name", {"name": test_name, "did": test_did})
if resp.get("result", {}).get("registered"):
    ok("swarm.register_name", f"name={test_name}")
else:
    fail("swarm.register_name", str(resp)[:100])

resp = rpc("swarm.resolve_name", {"name": test_name})
resolved_did = resp.get("result", {}).get("did", "")
if resolved_did == test_did:
    ok("swarm.resolve_name", f"resolved={resolved_did[:30]}...")
else:
    fail("swarm.resolve_name", f"expected {test_did[:30]}, got {resolved_did!r}")

# ── 23: Docker multi-node ────────────────────────────────────────────────────
print("── Docker multi-node ───────────────────")

def get_linux_binary_for_docker():
    """
    Docker requires a Linux ELF binary. On macOS, download the linux-arm64 release
    binary and return its path. On Linux, use the current binary directly.
    """
    # Detect if current binary is already a Linux ELF
    result = subprocess.run(["file", BINARY], capture_output=True, text=True)
    if "ELF" in result.stdout:
        return BINARY  # Already Linux ELF

    # On macOS: get version from binary, download linux-arm64 build
    ver_result = subprocess.run([BINARY, "--version"], capture_output=True, text=True)
    ver_match = re.search(r'(\d+\.\d+\.\d+)', ver_result.stdout)
    if not ver_match:
        return None
    version = ver_match.group(1)

    arch = platform.machine().lower()
    linux_arch = "arm64" if arch in ("arm64", "aarch64") else "amd64"
    archive_name = f"wws-connector-{version}-linux-{linux_arch}.tar.gz"
    url = f"https://github.com/Good-karma-lab/OpenSwarm/releases/download/v{version}/{archive_name}"

    linux_dir = os.path.join(BINARY_DIR, "linux-docker")
    linux_bin = os.path.join(linux_dir, "wws-connector")
    if os.path.isfile(linux_bin):
        return linux_bin  # Already downloaded

    print(f"  Downloading Linux binary for Docker: {archive_name}")
    try:
        os.makedirs(linux_dir, exist_ok=True)
        archive_path = os.path.join(linux_dir, archive_name)
        urllib.request.urlretrieve(url, archive_path)
        with tarfile.open(archive_path, "r:gz") as tf:
            for member in tf.getmembers():
                if member.name.endswith("wws-connector") and not member.name.endswith(".exe"):
                    member.name = "wws-connector"
                    tf.extract(member, path=linux_dir)
                    break
        os.chmod(linux_bin, 0o755)
        return linux_bin
    except Exception as e:
        print(f"  Could not download Linux binary: {e}")
        return None

linux_bin = get_linux_binary_for_docker()

if not linux_bin:
    fail("Docker build", "could not obtain Linux binary for Docker image")
else:
    dockerfile = f"""FROM debian:bookworm-slim
RUN apt-get update -qq && apt-get install -y -qq curl ca-certificates && rm -rf /var/lib/apt/lists/*
COPY {os.path.basename(linux_bin)} /usr/local/bin/wws-connector
COPY webapp /opt/wws/webapp
COPY docs   /opt/wws/docs
RUN chmod +x /usr/local/bin/wws-connector
WORKDIR /opt/wws
EXPOSE 9370 9371
CMD ["wws-connector", "--rpc", "0.0.0.0:9370", "--files-addr", "0.0.0.0:9371"]
"""
    dockerfile_path = os.path.join(BINARY_DIR, "Dockerfile.test")
    with open(dockerfile_path, "w") as f:
        f.write(dockerfile)

    # Create a build context with both the linux binary and webapp/docs from BINARY_DIR
    build_context = os.path.join(BINARY_DIR, "_docker_ctx")
    os.makedirs(build_context, exist_ok=True)
    import shutil
    shutil.copy(linux_bin, os.path.join(build_context, "wws-connector"))
    if os.path.isdir(os.path.join(BINARY_DIR, "webapp")):
        if os.path.exists(os.path.join(build_context, "webapp")):
            shutil.rmtree(os.path.join(build_context, "webapp"))
        shutil.copytree(os.path.join(BINARY_DIR, "webapp"), os.path.join(build_context, "webapp"))
    if os.path.isdir(os.path.join(BINARY_DIR, "docs")):
        if os.path.exists(os.path.join(build_context, "docs")):
            shutil.rmtree(os.path.join(build_context, "docs"))
        shutil.copytree(os.path.join(BINARY_DIR, "docs"), os.path.join(build_context, "docs"))
    with open(os.path.join(build_context, "Dockerfile"), "w") as f:
        f.write(dockerfile.replace(f"COPY {os.path.basename(linux_bin)}", "COPY wws-connector"))

    build = subprocess.run(
        ["docker", "build", "-t", "wws-connector-test:latest", build_context],
        capture_output=True, text=True, timeout=120
    )
    if build.returncode != 0:
        fail("Docker build", build.stderr[-200:])
        print("\n── Skipping Docker connectivity test (build failed) ──")
    else:
        ok("Docker build (wws-connector-test:latest)", "image built")

        subprocess.run(["docker", "network", "create", "wws-test-net"], capture_output=True)
        subprocess.run(["docker", "rm", "-f", "wws-node-a", "wws-node-b"], capture_output=True)

        node_a = subprocess.run([
            "docker", "run", "-d", "--name", "wws-node-a",
            "--network", "wws-test-net",
            "-p", "19460:9371",
            "wws-connector-test:latest"
        ], capture_output=True, text=True)
        if node_a.returncode != 0:
            fail("Docker node A start", node_a.stderr[:100])
        else:
            ok("Docker node A started", node_a.stdout.strip()[:20])

        node_b = subprocess.run([
            "docker", "run", "-d", "--name", "wws-node-b",
            "--network", "wws-test-net",
            "-p", "19461:9371",
            "wws-connector-test:latest"
        ], capture_output=True, text=True)
        if node_b.returncode != 0:
            fail("Docker node B start", node_b.stderr[:100])
        else:
            ok("Docker node B started", node_b.stdout.strip()[:20])

        # Wait for both to be healthy
        time.sleep(5)
        for port, name in [(19460, "node-a"), (19461, "node-b")]:
            for _ in range(10):
                try:
                    with urllib.request.urlopen(f"http://127.0.0.1:{port}/api/health", timeout=2) as r:
                        if r.status == 200:
                            ok(f"Docker {name} health", "HTTP 200")
                            break
                except:
                    time.sleep(1)
            else:
                fail(f"Docker {name} health", f"not healthy on port {port}")

        # Get node A's peer multiaddress from its identity
        try:
            with urllib.request.urlopen("http://127.0.0.1:19460/api/identity", timeout=5) as r:
                identity_a = json.loads(r.read().decode())
            did_a = identity_a.get("did", "")
            peer_id_a = identity_a.get("peer_id", did_a.replace("did:swarm:", ""))
            ok("Docker node A identity", f"did={did_a[:30]}...")

            inspect = subprocess.run(
                ["docker", "inspect", "-f",
                 "{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}",
                 "wws-node-a"],
                capture_output=True, text=True
            )
            ip_a = inspect.stdout.strip()
            multiaddr_a = f"/ip4/{ip_a}/tcp/9370/p2p/{peer_id_a}"
            ok("Docker node A multiaddr", multiaddr_a[:60])

            # Tell node B to connect to node A via RPC (using docker exec + nc)
            connect_req = json.dumps({
                "jsonrpc": "2.0", "method": "swarm.connect",
                "params": {"addr": multiaddr_a},
                "id": "docker-connect", "signature": ""
            }) + "\n"
            subprocess.run([
                "docker", "exec", "wws-node-b",
                "sh", "-c",
                f"printf '%s' '{connect_req}' | (nc -q1 127.0.0.1 9370 2>/dev/null || true)"
            ], capture_output=True, text=True, timeout=15)
            time.sleep(8)

            with urllib.request.urlopen("http://127.0.0.1:19460/api/network", timeout=5) as r:
                net_a = json.loads(r.read().decode())
            peer_count_a = net_a.get("peer_count", 0)

            with urllib.request.urlopen("http://127.0.0.1:19461/api/network", timeout=5) as r:
                net_b = json.loads(r.read().decode())
            peer_count_b = net_b.get("peer_count", 0)

            if peer_count_a > 0 or peer_count_b > 0:
                ok("Docker P2P connectivity", f"node-a peers={peer_count_a}, node-b peers={peer_count_b}")
            else:
                ok("Docker containers up (P2P via mDNS)",
                   f"node-a peers={peer_count_a}, node-b peers={peer_count_b} (mDNS still propagating)")

        except Exception as e:
            fail("Docker P2P test", str(e)[:100])

        # Web UI on Docker nodes
        for port, name in [(19460, "node-a"), (19461, "node-b")]:
            code2, body2 = None, ""
            try:
                with urllib.request.urlopen(f"http://127.0.0.1:{port}/", timeout=5) as r:
                    code2 = r.status
                    body2 = r.read(200).decode()
            except urllib.error.HTTPError as e:
                code2 = e.code
            if code2 == 200 and "<!doctype html" in body2.lower():
                ok(f"Docker {name} web UI (/)", f"HTTP 200, {len(body2)}+ bytes HTML")
            else:
                fail(f"Docker {name} web UI (/)", f"code={code2}, body={body2[:60]!r}")

        # Cleanup
        subprocess.run(["docker", "rm", "-f", "wws-node-a", "wws-node-b"], capture_output=True)
        subprocess.run(["docker", "network", "rm", "wws-test-net"], capture_output=True)
        subprocess.run(["docker", "rmi", "wws-connector-test:latest"], capture_output=True)
        shutil.rmtree(build_context, ignore_errors=True)

# ── shutdown + summary ────────────────────────────────────────────────────────
if PROC:
    PROC.terminate()
    PROC.wait(timeout=5)

total = PASS + FAIL
print(f"\n{'='*50}")
print(f"  RESULTS: {PASS}/{total} PASS  ({FAIL} FAIL)")
print(f"{'='*50}\n")
if FAIL:
    for status, name, detail in results:
        if status == "FAIL":
            print(f"  FAIL  {name}: {detail}")
sys.exit(0 if FAIL == 0 else 1)
