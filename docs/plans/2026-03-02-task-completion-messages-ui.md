# Task Completion, Synthesis, Messages Content, and UI Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Fix multi-tier synthesis propagation end-to-end; expose direct P2P messages (inbox + outbox) of the local agent in the Messages panel; BottomTray shows only root tasks; subtask table and DAG are clickable; add Result tab to TaskDetailPanel.

**Architecture:**
- **All swarm members are equal agents.** There is no "coordinator" role. An initiating agent (or its human operator via UI debug flow) submits a task to the swarm. The swarm self-organizes: a board forms, agents propose plans, deliberate, vote, and the winning plan is decomposed into subtasks. Subtasks are recursively handled the same way. The Messages panel shows only direct P2P messages (DMs) sent/received by the local connector's agent — not protocol traffic.
- **Two distinct propagation paths** — the connector handles one, agents handle the other:
  - **Status propagation (connector, automatic)**: when all siblings of a task reach `Completed` or `PendingReview`, the connector immediately marks their parent `Completed` and recurses up the chain. Works to any depth.
  - **Content synthesis (agents, explicit)**: `aggregate_subtask_results` only concatenates CID references (`subtask:X -> cid:abc\n...`) — it is a Merkle proof chain, NOT semantic text. `task_result_text` for parent tasks is only populated when an agent explicitly calls `swarm.submit_result` with `content: "..."` and `is_synthesis: true`. Agents at each board level are responsible for reading their children's results (`swarm.get_task`), synthesizing via LLM, and submitting the synthesis. The connector accepts `is_synthesis: true` submissions for already-Completed tasks, so synthesis can happen after the status has propagated.
- **Five bugs in the current flow**: (A) field name mismatch in submit_result; (B) P2P handler drops synthesis results from non-assignees; (C) content text stripped from P2P broadcast; (D) get_task RPC missing result_text; (E) P2P handler records deliberation message before populating text.
- Frontend changes: filter root tasks, click handlers, new Result tab, DM-only Messages panel.

IMPORTANT: connector - is just a gate to WWS for agents! all APIs of connector must be used only by agents (or UI)!
Node with UI must have a connected agent as well!
all tasks must be processed by agents! no scripts, no fallback implementation! no direct lolm calls from connectors!
While testing end-to-end - run a bunch of connectors and run 1 real AI agent (e.g. opencode, claude-code, or phase3_agent.py backed by a real LLM) per each connector.

---

## Task 1: Fix Field Name Mismatch (connector accepts both `content` and `result_text`)

**Root cause (Bug A):** The connector RPC handler (`rpc_server.rs` ~line 1277) reads only `"content"` from `submit_result` params. Some AI agents may send `"result_text"` (a common naming convention). If neither field is accepted, `task_result_text` is never populated and synthesis cannot proceed.

**Fix strategy:** Make the connector accept **both** `"content"` and `"result_text"` (with `"content"` taking precedence). This way any real AI agent using either field name works correctly without requiring changes to agent implementations.

**Files:**
- Modify: `crates/openswarm-connector/src/rpc_server.rs:~1277`

**Step 1: Write the failing test**

Add a test in `rpc_server.rs` confirming that a `submit_result` call with `"result_text"` DOES store text (currently fails), and one with `"content"` also works:

```rust
#[tokio::test]
async fn test_submit_result_stores_result_text_field() {
    let state = make_minimal_state();
    let network_handle = make_test_network_handle();
    let inject_params = serde_json::json!({"task_id":"t-rtext","description":"test","tier_level":2,"epoch":1});
    let _ = handle_inject_task(Some("1".into()), &inject_params, &state, &network_handle).await;
    {
        let mut s = state.write().await;
        if let Some(t) = s.task_details.get_mut("t-rtext") {
            t.parent_task_id = Some("parent-x".to_string());
        }
    }
    let params = serde_json::json!({
        "task_id": "t-rtext",
        "agent_id": "did:swarm:test-self",
        "result_text": "The answer is 42.",
        "artifact": {}
    });
    let resp = handle_submit_result(Some("2".into()), &params, &state, &network_handle).await;
    assert!(resp.error.is_none());
    let s = state.read().await;
    assert_eq!(s.task_result_text.get("t-rtext").map(|s| s.as_str()), Some("The answer is 42."));
}

#[tokio::test]
async fn test_submit_result_stores_content_field() {
    let state = make_minimal_state();
    let network_handle = make_test_network_handle();
    let inject_params = serde_json::json!({"task_id":"t-content","description":"test","tier_level":2,"epoch":1});
    let _ = handle_inject_task(Some("1".into()), &inject_params, &state, &network_handle).await;
    {
        let mut s = state.write().await;
        if let Some(t) = s.task_details.get_mut("t-content") {
            t.parent_task_id = Some("parent-x".to_string());
        }
    }
    let params = serde_json::json!({
        "task_id": "t-content",
        "agent_id": "did:swarm:test-self",
        "content": "The answer is 42.",
        "artifact": {}
    });
    let resp = handle_submit_result(Some("3".into()), &params, &state, &network_handle).await;
    assert!(resp.error.is_none());
    let s = state.read().await;
    assert_eq!(s.task_result_text.get("t-content").map(|s| s.as_str()), Some("The answer is 42."));
}
```

**Step 2: Run tests to confirm first fails, second passes**

Run: `~/.cargo/bin/cargo test -p openswarm-connector test_submit_result_stores 2>&1 | tail -20`

Expected: `test_submit_result_stores_result_text_field` FAILS, `test_submit_result_stores_content_field` PASSES.

**Step 3: Fix connector to accept both field names**

In `rpc_server.rs` ~line 1277, replace:

```rust
let content_text = params
    .get("content")
    .and_then(|v| v.as_str())
    .unwrap_or("")
    .to_string();
```

With:

```rust
let content_text = params
    .get("content")
    .or_else(|| params.get("result_text"))
    .and_then(|v| v.as_str())
    .unwrap_or("")
    .to_string();
```

**Step 4: Run tests to confirm both pass**

Run: `~/.cargo/bin/cargo test -p openswarm-connector test_submit_result_stores 2>&1 | tail -20`

Expected: both PASS.

**Step 5: Commit**

```bash
git add crates/openswarm-connector/src/rpc_server.rs
git commit -m "fix(connector): accept both 'content' and 'result_text' fields in submit_result"
```

---

## Task 2: Fix P2P ResultSubmission Handler (three bugs)

**Context:** When an agent calls `swarm.submit_result` via RPC, the connector updates its own state AND broadcasts a `ResultSubmission` P2P message so every other connector in the swarm can observe it. There are three independent bugs in this path.

---

**Bug B — P2P handler silently drops synthesis results**

Location: `connector.rs:1673`

```rust
// CURRENT (broken):
if task.assigned_to.as_ref() != Some(&params.agent_id) {
    // "Ignoring late result ..."
    return;
}
```

The P2P handler rejects any result where the submitting agent doesn't match the task's `assigned_to`. But synthesis results are legitimately submitted by the coordinator (not the executor). Two cases that should be accepted but are not:
- `params.is_synthesis = true` — coordinator synthesizing subtask results
- `task.assigned_to = None` — task was never explicitly assigned

The RPC handler (`rpc_server.rs:~1143`) already handles both cases correctly. The P2P handler is just missing those conditions.

---

**Bug C — `content` text is stripped from the P2P broadcast**

Location: `rpc_server.rs:1376–1380`

```rust
// CURRENT (broken):
let msg = SwarmMessage::new(
    ProtocolMethod::ResultSubmission.as_str(),
    serde_json::to_value(&submission).unwrap_or_default(),  // ← only struct fields
    String::new(),
);
```

`ResultSubmissionParams` struct has no `content` field. The result text is read separately from the raw JSON params and stored in `state.task_result_text` locally — but it is never added to the P2P message. Observer connectors receive the broadcast, try `raw_params.get("content")` (line 1751), find nothing, and never populate their own `task_result_text`. The UI on those nodes shows no result text.

---

**Bug E — deliberation message recorded before text is available (ordering)**

Location: `connector.rs:1706–1757`

The P2P handler first tries to record the synthesis deliberation message by reading `state.task_result_text.get(&params.task_id)` (line 1707), then at the very end inserts the content into `task_result_text` (line 1751). At observer nodes, `task_result_text` has no entry for this task yet — so the deliberation message is recorded with empty content, even after Bug C is fixed. The insert must happen before the recording.

---

**Files:**
- Modify: `crates/openswarm-connector/src/connector.rs:1673, 1706–1757`
- Modify: `crates/openswarm-connector/src/rpc_server.rs:1376–1380`

**Step 1: Fix Bug B — add missing conditions to P2P assignee check**

In `connector.rs`, replace lines 1673–1682:

```rust
// BEFORE:
if task.assigned_to.as_ref() != Some(&params.agent_id) {
    state.push_log(
        LogCategory::Task,
        format!(
            "Ignoring late result for task {} from replaced assignee {}",
            params.task_id, params.agent_id
        ),
    );
    return;
}
```

With:

```rust
// AFTER:
let assignee_ok = params.is_synthesis
    || task.assigned_to.is_none()
    || task.assigned_to.as_ref() == Some(&params.agent_id);
if !assignee_ok {
    state.push_log(
        LogCategory::Task,
        format!(
            "Ignoring result for task {} from non-assignee {}",
            params.task_id, params.agent_id
        ),
    );
    return;
}
```

**Step 2: Fix Bug C — inject `content` into P2P broadcast**

In `rpc_server.rs`, replace lines 1376–1380:

```rust
// BEFORE:
let msg = SwarmMessage::new(
    ProtocolMethod::ResultSubmission.as_str(),
    serde_json::to_value(&submission).unwrap_or_default(),
    String::new(),
);
```

With:

```rust
// AFTER:
let mut p2p_payload = serde_json::to_value(&submission).unwrap_or_default();
if let Some(obj) = p2p_payload.as_object_mut() {
    let content_val = params.get("content")
        .or_else(|| params.get("result_text"))
        .cloned();
    if let Some(cv) = content_val {
        obj.insert("content".to_string(), cv);
    }
}
let msg = SwarmMessage::new(
    ProtocolMethod::ResultSubmission.as_str(),
    p2p_payload,
    String::new(),
);
```

**Step 3: Fix Bug E — move `task_result_text` insert before deliberation recording**

In `connector.rs`, the current order inside the P2P handler is:
1. Record deliberation message using `state.task_result_text.get(...)` (line 1706)
2. Insert content into `task_result_text` (line 1751)

Move the insert block (lines 1751–1757) to BEFORE the deliberation message block (line 1706). After the fix the order should be:

```rust
// 1. Populate task_result_text from incoming content (moved up from end of handler)
if let Some(content) = raw_params.get("content").and_then(|v| v.as_str()) {
    if !content.trim().is_empty() {
        state
            .task_result_text
            .insert(params.task_id.clone(), content.to_string());
    }
}

// 2. Now record synthesis deliberation message (text is available now)
if let Some(text) = state.task_result_text.get(&params.task_id).cloned() {
    if !text.is_empty() {
        state.deliberation_messages.entry(params.task_id.clone()).or_default().push(DeliberationMessage {
            id: uuid::Uuid::new_v4().to_string(),
            task_id: params.task_id.clone(),
            timestamp: chrono::Utc::now(),
            speaker: params.agent_id.clone(),
            round: 3,
            message_type: DeliberationType::SynthesisResult,
            content: text,
            referenced_plan_id: None,
            critic_scores: None,
        });
    }
}

// ... rest of handler (DAG, logs) unchanged ...
```

**Step 4: Run all tests**

Run: `~/.cargo/bin/cargo test --workspace 2>&1 | tail -20`

Expected: all pass.

**Step 5: Commit**

```bash
git add crates/openswarm-connector/src/connector.rs crates/openswarm-connector/src/rpc_server.rs
git commit -m "fix(connector): fix P2P ResultSubmission — assignee_ok logic, propagate content, fix deliberation ordering"
```

---

## Task 3: Expose result_text in swarm.get_task Response

**Root cause (Bug D):** `handle_get_task` (rpc_server.rs ~line 1509) returns only `{"task": ..., "is_pending": ...}`. The `task_result_text` map (populated by `submit_result`) is never included. A coordinator AI agent calling `swarm.get_task` to read subtask results receives `null` for any result text, making synthesis impossible.

This is the connector-side prerequisite for coordinator synthesis. The coordinator AI agent is responsible for:
1. Calling `swarm.get_task` for each subtask ID to read the results
2. Using its own LLM reasoning to synthesize the results
3. Calling `swarm.submit_result` with `is_synthesis: true` and `content: "<synthesis>"` for the parent task

The connector only needs to expose the data — no synthesis logic belongs in the connector.

**Files:**
- Modify: `crates/openswarm-connector/src/rpc_server.rs:~1509–1515`

**Step 1: Write failing test**

Add in `rpc_server.rs` tests block:

```rust
#[tokio::test]
async fn test_get_task_includes_result_text() {
    let state = make_minimal_state();
    let network_handle = make_test_network_handle();
    // Inject a task
    let inject_params = serde_json::json!({"task_id":"t-rt","description":"test task","tier_level":2,"epoch":1});
    let _ = handle_inject_task(Some("1".into()), &inject_params, &state, &network_handle).await;
    {
        let mut s = state.write().await;
        if let Some(t) = s.task_details.get_mut("t-rt") {
            t.parent_task_id = Some("parent-x".to_string());
        }
    }
    // Submit a result with content
    let submit_params = serde_json::json!({
        "task_id": "t-rt",
        "agent_id": "did:swarm:test-self",
        "content": "Result: 42 is the answer.",
        "artifact": {}
    });
    let _ = handle_submit_result(Some("2".into()), &submit_params, &state, &network_handle).await;

    // Now get_task must include result_text
    let get_params = serde_json::json!({"task_id": "t-rt"});
    let resp = handle_get_task(Some("3".into()), &get_params, &state).await;
    assert!(resp.error.is_none());
    let result_text = resp.result.as_ref()
        .and_then(|r| r.get("result_text"))
        .and_then(|v| v.as_str());
    assert_eq!(result_text, Some("Result: 42 is the answer."));
}
```

**Step 2: Run test to confirm it fails**

Run: `~/.cargo/bin/cargo test -p openswarm-connector test_get_task_includes_result_text 2>&1 | tail -10`

Expected: FAIL (`result_text` not present in response).

**Step 3: Add result_text to handle_get_task response**

Find `handle_get_task` at ~line 1509:

```rust
SwarmResponse::success(
    id,
    serde_json::json!({
        "task": task,
        "is_pending": state.task_set.contains(&task.task_id),
    }),
)
```

Replace with:

```rust
let result_text = state.task_result_text.get(task_id).cloned();
SwarmResponse::success(
    id,
    serde_json::json!({
        "task": task,
        "is_pending": state.task_set.contains(&task.task_id),
        "result_text": result_text,
    }),
)
```

**Step 4: Run test to confirm it passes**

Run: `~/.cargo/bin/cargo test -p openswarm-connector test_get_task_includes_result_text 2>&1 | tail -10`

Expected: PASS.

**Step 5: Run all tests**

Run: `~/.cargo/bin/cargo test --workspace 2>&1 | tail -20`

Expected: all pass.

**Step 6: Commit**

```bash
git add crates/openswarm-connector/src/rpc_server.rs
git commit -m "fix(rpc): expose result_text in swarm.get_task response so coordinator agents can read subtask results"
```

---

## Task 4: Add Outbox + `/api/conversations` Endpoint (DMs only)

**Problem:**
- `handle_send_message` fires-and-forgets. No local record of sent messages.
- There is no endpoint that exposes the DMs (direct messages) sent and received by this node's agent.

**Scope:** This endpoint exposes only direct P2P messages (`swarm.send_message` / inbox) for the local agent. Protocol traffic — deliberation messages, proposals, critiques, votes — is **not** shown here; those are already visible in the TaskDetailPanel Deliberation tab per task. The Messages panel is for agent-to-agent communication, not swarm protocol internals.

**Files:**
- Modify: `crates/openswarm-connector/src/connector.rs` — add `outbox` field to `ConnectorState`, initialize it
- Modify: `crates/openswarm-connector/src/operator_console.rs` — add `outbox: Vec::new()` to all `ConnectorState` literals
- Modify: `crates/openswarm-connector/src/rpc_server.rs:~2451` — store in outbox before publish
- Modify: `crates/openswarm-connector/src/file_server.rs` — add `ConversationItem`, `api_conversations`, route

**Step 1: Add `outbox` to ConnectorState**

In `connector.rs`, after `pub inbox: Vec<InboxMessage>,` (line ~280), add:

```rust
/// Outbox of direct messages sent by this agent.
pub outbox: Vec<InboxMessage>,
```

In the main `ConnectorState` initializer (connector.rs ~line 776), after `inbox: Vec::new(),`, add:

```rust
outbox: Vec::new(),
```

Do the same for every other `ConnectorState { ... }` literal:
- `connector.rs` test helper (~line 3324)
- `operator_console.rs` (3 occurrences at lines ~1370, ~1455, ~1548+)

**Step 2: Record outbox in handle_send_message**

In `rpc_server.rs::handle_send_message`, after extracting `from` and `to` (after line ~2436), record before publish:

```rust
{
    let mut s = state.write().await;
    s.outbox.push(InboxMessage {
        from: from.clone(),
        to: to.clone(),
        content: content.clone(),
        timestamp: chrono::Utc::now(),
    });
}
```

**Step 3: Write test for DirectMessage serialization**

At bottom of `file_server.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_direct_message_serializes() {
        let item = DirectMessage {
            timestamp: chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
            direction: "received".into(),
            from: "did:swarm:abc".into(),
            to: "did:swarm:xyz".into(),
            content: "Hello from agent.".into(),
        };
        let json = serde_json::to_string(&item).unwrap();
        assert!(json.contains("received"));
        assert!(json.contains("Hello from agent"));
    }
}
```

**Step 4: Run test to verify it fails**

Run: `~/.cargo/bin/cargo test -p openswarm-connector test_direct_message_serializes 2>&1 | tail -10`

Expected: FAIL (`DirectMessage` not defined yet)

**Step 5: Add DirectMessage and api_conversations**

In `file_server.rs`, after the `use` block at the top:

```rust
/// A direct P2P message (sent or received) for the local agent.
/// Protocol traffic (proposals, critiques, votes) is NOT included here.
#[derive(serde::Serialize)]
pub struct DirectMessage {
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// "received" (inbox) or "sent" (outbox)
    pub direction: String,
    pub from: String,
    pub to: String,
    pub content: String,
}
```

Add the handler near other `api_*` functions:

```rust
async fn api_conversations(State(web): State<WebState>) -> Json<serde_json::Value> {
    let state = web.state.read().await;
    let mut items: Vec<DirectMessage> = Vec::new();

    for m in &state.inbox {
        items.push(DirectMessage {
            timestamp: m.timestamp,
            direction: "received".into(),
            from:      m.from.clone(),
            to:        m.to.clone(),
            content:   m.content.clone(),
        });
    }

    for m in &state.outbox {
        items.push(DirectMessage {
            timestamp: m.timestamp,
            direction: "sent".into(),
            from:      m.from.clone(),
            to:        m.to.clone(),
            content:   m.content.clone(),
        });
    }

    items.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    items.truncate(200);
    let count = items.len();
    Json(serde_json::json!({ "conversations": items, "count": count }))
}
```

Register route in the `Router::new()` builder:

```rust
.route("/api/conversations", get(api_conversations))
```

**Step 6: Run test to verify it passes**

Run: `~/.cargo/bin/cargo test -p openswarm-connector test_direct_message_serializes 2>&1 | tail -10`

Expected: PASS

**Step 7: Build check**

Run: `~/.cargo/bin/cargo build -p openswarm-connector 2>&1 | tail -20`

Expected: no errors.

**Step 8: Commit**

```bash
git add crates/openswarm-connector/src/connector.rs \
        crates/openswarm-connector/src/operator_console.rs \
        crates/openswarm-connector/src/rpc_server.rs \
        crates/openswarm-connector/src/file_server.rs
git commit -m "feat(api): add outbox to ConnectorState and /api/conversations endpoint"
```

---

## Task 5: Rewrite MessagesPanel + Wire App.jsx

**Files:**
- Modify: `webapp/src/api/client.js`
- Modify: `webapp/src/App.jsx`
- Modify: `webapp/src/components/MessagesPanel.jsx`

**Step 1: Add conversations API method**

In `client.js`, add to the `api` object:

```js
conversations: () => fetchJson('/api/conversations'),
```

**Step 2: Update App.jsx**

Add state: `const [conversations, setConversations] = useState([])`

In `refresh`, add `api.conversations()` to the `Promise.all` and add `setConversations(conv.conversations)`:

```js
const [v, m, t, ag, tp, a, au, hl, conv] = await Promise.all([
  api.voting(),
  api.messages(),
  api.tasks(),
  api.agents(),
  api.topology(),
  api.audit(),
  api.authStatus(),
  api.holons().catch(() => []),
  api.conversations().catch(() => ({ conversations: [], count: 0 })),
])
// existing setters ...
setConversations(conv?.conversations ?? [])
```

Pass to MessagesPanel:

```jsx
{panel?.type === 'messages' && (
  <SlidePanel title="P2P Messages" onClose={closePanel}>
    <MessagesPanel conversations={conversations} />
  </SlidePanel>
)}
```

**Step 3: Rewrite MessagesPanel.jsx**

Shows only direct messages for the local agent. Protocol traffic (deliberations, votes, plans) belongs in the TaskDetailPanel Deliberation tab, not here.

```jsx
function scrubId(s) {
  return String(s || '').replace(/did:swarm:[A-Za-z0-9_-]+/g, m => m.slice(-8))
}

export default function MessagesPanel({ conversations }) {
  if (!conversations.length) {
    return (
      <div style={{ padding: 16, color: 'var(--text-muted)', fontFamily: 'var(--font-mono)', fontSize: 12 }}>
        No direct messages yet.
      </div>
    )
  }

  return (
    <div>
      <div className="detail-section-title">
        Direct Messages ({conversations.length})
      </div>
      <div className="log-box" style={{ fontFamily: 'var(--font-mono)', fontSize: 11, maxHeight: '70vh' }}>
        {conversations.map((c, i) => {
          const sent = c.direction === 'sent'
          return (
            <div key={i} style={{
              marginBottom: 10,
              borderBottom: '1px solid var(--border)',
              paddingBottom: 8,
              borderLeft: `3px solid ${sent ? '#ffaa00' : 'var(--teal)'}`,
              paddingLeft: 10,
            }}>
              <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap', marginBottom: 4, alignItems: 'center' }}>
                <span style={{ color: 'var(--text-muted)', fontSize: 10 }}>
                  [{new Date(c.timestamp).toLocaleTimeString()}]
                </span>
                <span style={{
                  color: '#fff',
                  background: sent ? '#6b4400' : '#004433',
                  borderRadius: 3,
                  padding: '1px 6px',
                  fontSize: 10,
                  textTransform: 'uppercase',
                }}>
                  {sent ? 'sent' : 'received'}
                </span>
                <span style={{ color: sent ? '#ffaa00' : 'var(--teal)' }}>
                  {scrubId(c.from)}
                </span>
                <span style={{ color: 'var(--text-muted)' }}>→</span>
                <span style={{ color: 'var(--text)' }}>
                  {scrubId(c.to)}
                </span>
              </div>
              <div style={{ color: 'var(--text)', whiteSpace: 'pre-wrap', lineHeight: 1.5, wordBreak: 'break-word' }}>
                {c.content}
              </div>
            </div>
          )
        })}
      </div>
    </div>
  )
}
```

**Step 4: Build webapp**

Run: `cd /Users/aostapenko/Work/OpenSwarm/webapp && npm run build 2>&1 | tail -15`

Expected: success.

**Step 5: Commit**

```bash
git add webapp/src/api/client.js webapp/src/App.jsx webapp/src/components/MessagesPanel.jsx
git commit -m "feat(ui): rewrite MessagesPanel — show only direct P2P messages (sent/received) for local agent"
```

---

## Task 6: Filter BottomTray to Root Tasks Only

**Context:** The BottomTray Tasks column uses `buildTaskTree()` (added in v0.6.2) which shows subtasks indented. Only root tasks (those with no `parent_task_id` — i.e. tasks submitted by the initiating agent or operator) should appear here. Subtasks created during swarm decomposition are visible in the TaskDetailPanel DAG and subtask table when you open the parent task.

**Files:**
- Modify: `webapp/src/components/BottomTray.jsx:68–71`

**Step 1: Filter to root tasks**

Change:

```js
const taskList = tasks?.tasks || []
const taskTree = buildTaskTree(taskList)
```

To:

```js
const taskList = (tasks?.tasks || []).filter(t => !t.parent_task_id)
```

Replace all `taskTree` references in the render with `taskList`. Remove the `_depth`-based style block from the task item render:

```jsx
{taskList.slice(0, 30).map(t => (
  <div
    key={t.task_id}
    className="task-item"
    onClick={() => onTaskClick(t)}
  >
    <span className="task-status-dot" style={{ background: taskStatusColor(t.status) }} />
    <span className="task-id" style={{ fontFamily: 'var(--font-mono)', fontSize: 10 }}>
      {t.task_id.slice(0, 8)}…
    </span>
    <span className="task-desc">{t.description || t.task_id}</span>
    <span style={{ fontSize: 10, color: 'var(--text-muted)', flexShrink: 0 }}>{t.status}</span>
  </div>
))}
```

Also update the empty check: `{taskList.length === 0 && ...`

The `buildTaskTree` function and its helpers can remain in the file (they may be used elsewhere later) — just stop calling them here.

**Step 2: Build**

Run: `cd /Users/aostapenko/Work/OpenSwarm/webapp && npm run build 2>&1 | tail -10`

**Step 3: Commit**

```bash
git add webapp/src/components/BottomTray.jsx
git commit -m "feat(ui): filter BottomTray task list to Tier-1 root tasks only"
```

---

## Task 7: Make Subtasks Clickable in TaskDetailPanel (table + DAG)

**Problem:** OverviewTab shows a Subtasks table and a Task DAG (vis-network). Clicking a row or node does nothing. User wants clicking to open the task detail panel for that subtask.

**Files:**
- Modify: `webapp/src/App.jsx` — pass `onTaskClick` to `<TaskDetailPanel>`
- Modify: `webapp/src/components/TaskDetailPanel.jsx` — thread prop to OverviewTab, wire table rows and DAG click event

**Step 1: Pass onTaskClick from App.jsx**

In App.jsx, inside the `panel?.type === 'task'` render block, add the prop:

```jsx
<TaskDetailPanel
  taskId={panel.data.taskId}
  taskTrace={taskTrace}
  taskVoting={taskVoting}
  taskBallots={taskBallots}
  agents={agents}
  onTaskClick={openTaskPanel}
/>
```

**Step 2: Thread prop through TaskDetailPanel → OverviewTab**

In the main export signature:

```jsx
export default function TaskDetailPanel({ taskId, taskTrace, taskVoting, taskBallots, agents, onTaskClick }) {
```

Pass to OverviewTab:

```jsx
{activeTab === 'overview' && (
  <OverviewTab taskTrace={taskTrace} agents={agents} onTaskClick={onTaskClick} />
)}
```

**Step 3: Wire subtask table rows in OverviewTab**

The OverviewTab signature becomes:

```jsx
function OverviewTab({ taskTrace, agents, onTaskClick }) {
```

In the subtask `<table>` `<tbody>`, add onClick to each row. Find:

```jsx
{descendants.map(t => (
  <tr key={t.task_id}>
```

Change to:

```jsx
{descendants.map(t => (
  <tr
    key={t.task_id}
    style={{ cursor: 'pointer' }}
    onClick={() => onTaskClick && onTaskClick({ task_id: t.task_id })}
  >
```

**Step 4: Wire DAG vis-network click event**

In the `useEffect` that creates the Network (after `dagNet.current = new Network(...)`), add:

```js
dagNet.current.on('click', function(params) {
  if (params.nodes.length > 0 && onTaskClick) {
    onTaskClick({ task_id: params.nodes[0] })
  }
})
```

**Step 5: Build**

Run: `cd /Users/aostapenko/Work/OpenSwarm/webapp && npm run build 2>&1 | tail -10`

Expected: success.

**Step 6: Commit**

```bash
git add webapp/src/App.jsx webapp/src/components/TaskDetailPanel.jsx
git commit -m "feat(ui): make subtask table rows and DAG nodes clickable in TaskDetailPanel"
```

---

## Task 8: Add Result Tab to TaskDetailPanel

**Problem:** No dedicated tab showing agent answer text or synthesis result. `result_text` from the API timeline response is buried in OverviewTab inside the "Result Artifact" section, visible only when `result_artifact` exists.

**Files:**
- Modify: `webapp/src/components/TaskDetailPanel.jsx`

**Step 1: Add ResultTab component**

After the `DeliberationTab` function definition, add:

```jsx
function ResultTab({ taskTrace, agents }) {
  const agentsList = agents?.agents || []
  const task = taskTrace?.task
  const resultText = taskTrace?.result_text
  const subtasksWithResults = (taskTrace?.descendants || []).filter(d => d.result_text)

  if (!resultText && !subtasksWithResults.length) {
    return (
      <div style={{ color: 'var(--text-muted)', fontSize: 12, padding: '8px 0' }}>
        No result yet.
      </div>
    )
  }

  return (
    <div>
      {resultText && (
        <div className="detail-section">
          <div className="detail-section-title">
            Final Result
            {task?.assigned_to_name && (
              <span style={{ fontWeight: 400, color: 'var(--teal)', marginLeft: 8, fontSize: 11 }}>
                by {task.assigned_to_name}
              </span>
            )}
          </div>
          <div style={{
            background: 'var(--surface-2)',
            border: '1px solid var(--border)',
            borderRadius: 6,
            padding: '12px 16px',
            fontSize: 13,
            color: 'var(--text)',
            lineHeight: 1.6,
            whiteSpace: 'pre-wrap',
            wordBreak: 'break-word',
          }}>
            {resultText}
          </div>
        </div>
      )}

      {subtasksWithResults.length > 0 && (
        <div className="detail-section">
          <div className="detail-section-title">Subtask Results ({subtasksWithResults.length})</div>
          {subtasksWithResults.map(d => (
            <div key={d.task_id} style={{
              background: 'var(--surface-2)',
              border: '1px solid var(--border)',
              borderRadius: 6,
              padding: '10px 14px',
              marginBottom: 10,
            }}>
              <div style={{ display: 'flex', gap: 8, marginBottom: 6, flexWrap: 'wrap', alignItems: 'center' }}>
                <span style={{ fontFamily: 'var(--font-mono)', fontSize: 10, color: 'var(--text-muted)' }}>
                  {d.task_id.slice(0, 12)}…
                </span>
                <span style={{ fontSize: 11, color: 'var(--text)' }}>{d.description}</span>
                {d.assigned_to_name && (
                  <span style={{ fontSize: 10, color: 'var(--teal)', marginLeft: 'auto' }}>
                    {d.assigned_to_name}
                  </span>
                )}
              </div>
              <div style={{
                fontSize: 12,
                color: 'var(--text)',
                lineHeight: 1.5,
                whiteSpace: 'pre-wrap',
                wordBreak: 'break-word',
                borderLeft: '2px solid var(--border)',
                paddingLeft: 10,
              }}>
                {d.result_text}
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  )
}
```

**Step 2: Add tab to tab list**

Change:

```jsx
{['overview', 'voting', 'deliberation'].map(tab => (
```

To:

```jsx
{['overview', 'result', 'voting', 'deliberation'].map(tab => (
```

**Step 3: Add ResultTab render block**

After `{activeTab === 'deliberation' && ...}`, add:

```jsx
{activeTab === 'result' && (
  <ResultTab taskTrace={taskTrace} agents={agents} />
)}
```

**Step 4: Build**

Run: `cd /Users/aostapenko/Work/OpenSwarm/webapp && npm run build 2>&1 | tail -10`

Expected: success.

**Step 5: Commit**

```bash
git add webapp/src/components/TaskDetailPanel.jsx
git commit -m "feat(ui): add Result tab to TaskDetailPanel showing agent answer and subtask results"
```

---

## Task 9: Verify result_text data is complete end-to-end

**No code changes needed.** `api_task_timeline` in `file_server.rs` already correctly returns:
- `result_text` at the top level (from `state.task_result_text`) — consumed by `taskTrace?.result_text` in `ResultTab`
- `result_text` per descendant (also from `state.task_result_text`) — consumed by `taskTrace?.descendants[].result_text` in `ResultTab`

The UI's `taskTrace` is populated from `GET /api/tasks/:task_id/timeline` which is this endpoint.

**What this task does:** Run all tests and verify the end-to-end data flow after Tasks 1–8 are implemented. The bugs fixed in Tasks 1–2 (accepting both field names, injecting content into P2P broadcast, fixing deliberation ordering) are the blockers that caused `task_result_text` to be empty at observer nodes — the HTTP layer was correct all along.

**Step 1: Run all tests**

Run: `~/.cargo/bin/cargo test --workspace 2>&1 | tail -30`

Expected: all pass.

**Step 2: Smoke-check the HTTP endpoint**

With a running connector that has a completed task:

```bash
curl http://127.0.0.1:9371/api/tasks/<task_id>/timeline | python3 -m json.tool | grep -A2 "result_text"
```

Expected: `result_text` non-null at both top level and per-descendant entry.

**Step 3: No commit needed** — if tests pass, proceed to Task 10.

---

## Task 10: Bump Version, Build Release, Rebuild Docker

**Step 1:** Bump `Cargo.toml` version to `0.6.3`

**Step 2:** Build Rust release: `~/.cargo/bin/cargo build --release --bin wws-connector 2>&1 | tail -10`

**Step 3:** Build webapp: `cd webapp && npm run build 2>&1 | tail -10`

**Step 4:** Run all tests: `~/.cargo/bin/cargo test --workspace 2>&1 | tail -30`

**Step 5:** Rebuild Docker: `docker build -f docker/Dockerfile -t wws-connector:local . 2>&1 | tail -15`

**Step 6:** Restart swarm: `cd docker && docker compose down && docker compose up -d`

**Step 7:** Commit version bump:

```bash
git add Cargo.toml Cargo.lock
git commit -m "chore: bump version to v0.6.3"
```

---

## Verification

Run 1 real AI agent per connector node (e.g. `opencode-agent.sh` or Claude Code subagent). No hardcoded-response scripts.

1. Start 20-node Docker swarm + connect 1 real AI agent per connector node
2. Submit task via web UI (or initiating agent calls `inject_task`) → task appears in BottomTray (root tasks only, no subtasks)
3. Swarm self-organizes: board forms, agents propose plans, vote, winning plan is selected → subtasks created, visible in TaskDetailPanel Overview tab (clickable)
4. Click any subtask → detail panel opens for that subtask
5. Agents execute subtasks and call `swarm.submit_result` with `content` → when all siblings at a level are done, the connector auto-marks their parent `Completed` (status only, no text). Board members at each level read their children's results via `swarm.get_task`, synthesize via LLM, and call `swarm.submit_result` with `is_synthesis: true` and `content: "..."` for their parent task. This repeats up each level. Root task eventually transitions to `Completed` with synthesis text.
6. Click root task → Result tab shows synthesis text (set by the board member who synthesized at root level); Subtask Results section shows per-agent answers
7. Open Messages panel → shows only direct messages (sent/received) for the local agent — no protocol traffic
8. Agent sends DM → appears as "sent" on sender's connector; as "received" on recipient's connector
