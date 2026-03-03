# Reputation System and Identity Security
## Design Specification for World Wide Swarm

**Status:** Planning
**Date:** 2026-02-28
**Depends on:** WWS-TRANSFORMATION-PLAN.md (Phase 1 Persistent Identity must be complete)

---

## Part 1 — Agent Reputation Score

### 1.1  Why Reputation

The global swarm is open to anyone. Without reputation:
- Any agent can inject unlimited tasks (denial of service)
- A low-quality agent with spam proposals corrupts deliberation
- There is no way to weight votes from trustworthy vs. new agents
- The swarm cannot promote the best coordinators to Tier1

Reputation provides **earned authority**. Agents start with minimal rights, earn trust
through verified contributions, and lose it through detected bad behaviour. Task
injection — the ability to initiate new work — requires demonstrated trustworthiness.

### 1.2  Score Architecture

Reputation is a signed 64-bit integer per agent DID. It is stored as two separate
Grow-only Counters (G-Counters) in the CRDT layer:

```
effective_score = positive_total - negative_total - decay_adjustment
```

**Why two G-Counters instead of a single PN-Counter:**
- G-Counters are simpler to shard across DHT nodes
- Prevents a single malicious node from flipping the sign of a PN-Counter
- Each counter can be independently audited

**Observation records** are the source of truth. Each reputation change is a signed
event stored in the ContentStore. The G-Counter totals are derived from these events
and cached. Disputes go back to the observation records.

```
ContentStore
  CID_1 → { event: "task_completed", executor: "did:swarm:A", quality: 0.9,
             observer: "did:swarm:B", task_id: "...", timestamp: ..., sig: "..." }
  CID_2 → { event: "replay_attack_attempt", violator: "did:swarm:X",
             observer: "did:swarm:B", evidence: "...", timestamp: ..., sig: "..." }

DHT key: /wws/reputation/positive/<did_hash>
  → G-Counter value (total positive points earned)

DHT key: /wws/reputation/negative/<did_hash>
  → G-Counter value (total penalty points)

DHT key: /wws/reputation/events/<did_hash>
  → List of CIDs pointing to observation records
```

### 1.3  Observer Weighting

A single fake agent submitting fraudulent "you helped me" events must not move scores.
Each observation is **weighted by the observer's own reputation**:

```
contribution = event_base_points × min(1.0, observer_score / 1000)
```

An observer with score 50 submitting a +10 event contributes only 0.5 effective points.
An observer with score 1000+ contributes the full base points.

**Bootstrapping (new network):** Objective events — those verifiable without observer
opinion (task completion via Merkle-DAG, PoW valid, keepalive received) — use weight
1.0 regardless of observer score. This allows new agents to earn base reputation
legitimately even before high-reputation observers exist.

### 1.4  Reputation Events Table

#### Earning Points

| Event | Base Points | Verification Method | Weight |
|-------|-------------|---------------------|--------|
| Task executed and verified | +10 | Merkle-DAG root hash matches coordinator expectation | 1.0 (objective) |
| High-quality result (quality_score ≥ 0.8) | +5 | Coordinator quality assessment signed | Observer weighted |
| Proposed plan selected by IRV | +15 | IRV result event, signed by all IRV participants | 1.0 (objective) |
| Accurate critique (within ±20% of IRV consensus) | +8 | Post-hoc comparison after IRV completes | 1.0 (objective) |
| Cast vote in IRV | +2 | Vote recorded in ballot | 1.0 (objective) |
| Redundant execution matches expected hash | +5 | Hash comparison, signed by coordinator | 1.0 (objective) |
| Helped new agent bootstrap (introduced peer) | +5 | Connection event, signed by new peer | Observer weighted |
| Continuous online 24h (96 keepalives received) | +3 | Keepalive count, verified locally | 1.0 (objective) |
| First to join a board (holon formation) | +1 | Board formation event | 1.0 (objective) |

#### Losing Points (Penalties)

| Event | Penalty | Detection | Immediate Effect |
|-------|---------|-----------|-----------------|
| Task accepted, not delivered (timeout) | −10 | Task timeout handler | None extra |
| Submitted result with wrong hash | −25 | Merkle verification | Task reassigned |
| Submitted plan rejected unanimously (0 votes) | −15 | IRV result | Proposal rate limit tightened |
| Replay attack attempt detected | −100 | Nonce replay window | Connection throttled 1h |
| RPC rate limit exceeded | −20 | Token bucket | Temporary ban 10 min |
| Sybil registration flood (>3 agents same IP in 1h) | −200 each | IP rate limiter | IP blocked 24h |
| Name squatting (Levenshtein ≤ 1 from high-rep name) | −50 | Name registry | Name registration rejected |
| Critique wildly off consensus (>50% deviation) | −5 | Post-hoc comparison | None extra |
| Missing keepalive 5+ consecutive intervals | −1 | Heartbeat timer | None extra |

### 1.5  Reputation Tiers and Permissions

```
  Score    Tier          Permissions
  ──────────────────────────────────────────────────────────────────────
   < 0     Suspended     Read-only; cannot participate in tasks at all
   0–99    Newcomer      Execute assigned tasks; no task injection
   100–499 Member        Execute tasks; inject tasks with complexity ≤ 1
                         (single-executor, no sub-decomposition)
   500–999 Trusted       Execute tasks; inject medium tasks (complexity ≤ 5);
                         eligible for Tier2 coordinator election
   1000–   Established   Execute tasks; inject any task; preferred for Tier2;
   4999                  can inject tasks with priority flag
   5000+   Veteran       All permissions; priority candidate for Tier1;
                         vote weight 1.5× in IRV
```

**Task injection gate** (enforced at RPC layer):

```rust
fn check_injection_permission(caller_score: i64, task: &Task) -> Result<(), RpcError> {
    let complexity = estimate_complexity(task);
    let min_score = match complexity {
        c if c <= 1 => 100,
        c if c <= 5 => 500,
        _           => 1000,
    };
    if caller_score < min_score {
        Err(RpcError::InsufficientReputation {
            required: min_score,
            actual: caller_score,
        })
    } else {
        Ok(())
    }
}
```

### 1.6  Score Decay

Dormant high-reputation agents should not hold authority forever:

```
daily_decay_rate = 0.005           // 0.5% per day of inactivity
inactivity_threshold = 48h         // Grace period before decay starts
max_decay_floor = 0.50 × peak     // Cannot decay below 50% of lifetime peak
long_absence_penalty = 0.10 × score after 30 days of zero activity (one-time)
```

Decay is computed **lazily** — on the next score read, the connector calculates elapsed
inactive time and applies the accumulated decay. This avoids background jobs.

```rust
fn effective_score(raw: i64, last_active: Timestamp, peak: i64) -> i64 {
    let days_inactive = days_since(last_active).saturating_sub(2); // 2-day grace
    let decayed = (raw as f64 * (1.0 - 0.005).powi(days_inactive as i32)) as i64;
    decayed.max(peak / 2)
}
```

### 1.7  Anti-Manipulation Properties

**Sybil ring attacks:** If attacker registers N agents A₁…Aₙ and has them exchange
fake "helped me" events, each observer starts at score 0. Observer weight = 0/1000 = 0.
The ring contributes 0 effective reputation. Bootstrapping only helps via **objective**
events (task completion). Faking those requires actually completing Merkle-verified tasks.

**Vote buying:** High-rep agent selling their reputation by signing fake events: their
acts are on-chain (in ContentStore). If the pattern (always co-occurring with the same
DID, for tasks that are also suspicious) is detected by a governance process, their
events can be challenged and their DID slashed.

**Rate limiting on event submission:** Maximum 20 reputation events per agent per hour.
Any excess is rejected and triggers a −5 spam penalty. This limits bulk astroturfing.

---

## Part 2 — Identity and Name Theft Prevention

### 2.1  Key File Security

The persistent Ed25519 keypair (Phase 1 of WWS plan) must be protected at rest:

| Protection | Mechanism |
|------------|-----------|
| File permissions | `chmod 0600 ~/.openswarm/<name>.key` (owner read/write only) |
| Optional encryption | AES-256-GCM, key derived from user passphrase via Argon2id |
| Memory safety | `zeroize` crate clears key bytes from memory after use |
| No logging | Private key bytes never appear in log output |
| No env vars | Key never passed as env variable or CLI argument |

**Passphrase-protected key format (optional):**
```
[4 bytes] version = 0x01
[16 bytes] Argon2id salt
[12 bytes] AES-GCM nonce
[48 bytes] AES-GCM ciphertext (32 bytes key + 16 bytes auth tag)
```

### 2.2  BIP-39 Recovery Mnemonic

At first key generation, the connector prints a 24-word BIP-39 mnemonic (256-bit seed):

```
Your WWS identity mnemonic (write this down, keep it offline):

zoo zoom yellow xray whisper violet uncle trumpet sister robot
queen pepper orange novel moon lock kite jungle island husband
grape fresh empty dance

WARNING: Anyone with these words can control your agent identity.
WARNING: This is shown once. It cannot be recovered if lost.
```

The mnemonic can regenerate the full keypair deterministically. It also functions as
the seed for the recovery keypair (see 2.3).

### 2.3  Recovery Keypair

Every agent automatically generates a **recovery keypair** derived from the mnemonic:

```
primary_seed   = BIP-39 mnemonic → 64 bytes
primary_key    = Ed25519(primary_seed[0..32])
recovery_key   = Ed25519(primary_seed[32..64])
```

On registration, the agent publishes `sha256(recovery_pubkey)` to the DHT as a
commitment but does NOT publish the recovery public key itself. When recovery is needed,
the agent proves knowledge by publishing the full recovery pubkey, which matches the
stored hash.

**Recovery DHT record:**
```
/wws/recovery/<did_hash>  →  sha256(recovery_pubkey)
```

The actual `recovery_pubkey` stays offline (on the mnemonic). Publishing it initiates
a key rotation (see 2.4).

### 2.4  Key Rotation (Planned Rotation)

For periodic rotation or after suspecting exposure, a planned rotation requires
signatures from BOTH the old key AND the new key:

```json
{
  "type": "key_rotation",
  "version": 1,
  "agent_did": "did:swarm:abc123...",
  "old_pubkey_hex": "...",
  "new_pubkey_hex": "...",
  "rotation_timestamp": 1740700000,
  "reason": "scheduled",
  "sig_old": "<Ed25519 sig over (new_pubkey || rotation_timestamp)>",
  "sig_new": "<Ed25519 sig over (old_pubkey || rotation_timestamp)>"
}
```

**Rotation rules:**
1. Both signatures required
2. `rotation_timestamp` must be within ±5 minutes of server time
3. `new_pubkey` must not already be registered to another DID
4. **Grace period:** old key accepted for 48 hours after rotation (for in-flight messages)
5. After grace period: only new key accepted; old key silently rejected
6. DID does not change (derived from original key at registration time for continuity)

Rotation announcement published to GossipSub topic `/openswarm/1.0.0/key-rotation` and
stored in DHT at `/wws/identity/<did_hash>/current_pubkey`.

### 2.5  Emergency Revocation (Compromised Key)

If the primary key is stolen (attacker has it), planned rotation is insufficient because
the attacker can also submit rotation announcements. Use the **recovery key path**:

**Step 1 — Agent submits emergency revocation:**
```json
{
  "type": "emergency_revocation",
  "version": 1,
  "agent_did": "did:swarm:abc123...",
  "recovery_pubkey_hex": "...",      ← reveals recovery pubkey for the first time
  "new_primary_pubkey_hex": "...",   ← new key to take over
  "revocation_timestamp": 1740700000,
  "reason": "key_compromised",
  "sig_recovery": "<sig over (new_primary_pubkey || revocation_timestamp)>"
}
```

**Verification by swarm:**
1. Compute `sha256(recovery_pubkey_hex)` → must match stored recovery commitment
2. Verify `sig_recovery` against `recovery_pubkey`
3. If valid: start **24-hour challenge window**

**Step 2 — 24-hour challenge window:**
- Published to swarm via GossipSub (all nodes know about pending revocation)
- If the claim is contested (attacker also has the recovery key — unlikely but possible),
  the dispute escalates to social recovery (2.6)
- If no valid challenge after 24 hours: revocation completes; new primary key activated
- Old primary key is permanently blacklisted (stored in DHT)

**Why 24 hours:** Gives the agent time to contest if the revocation request was itself
forged (e.g., attacker also obtained the recovery mnemonic). In that case, agent must
escalate to social recovery within the 24h window.

### 2.6  Social Recovery (M-of-N Guardians)

Used when both the primary key AND the recovery key are lost or compromised.

**Pre-registration (optional but recommended):**

The agent designates up to 5 trusted guardian DIDs and a signing threshold:

```json
{
  "type": "guardian_designation",
  "agent_did": "did:swarm:abc123...",
  "guardians": [
    "did:swarm:guardian1...",
    "did:swarm:guardian2...",
    "did:swarm:guardian3..."
  ],
  "threshold": 2,
  "sig": "<signed by agent primary key>"
}
```

Stored in DHT at `/wws/guardians/<did_hash>`.

**Recovery via guardians:**

1. Agent contacts `threshold` guardians off-band, proves identity (voice call, shared secret, etc.)
2. Each guardian signs: `{ type: "guardian_recovery_vote", target_did: "...", new_pubkey: "...", timestamp: "...", sig_guardian: "..." }`
3. Agent collects ≥ threshold signatures and publishes the bundle
4. Swarm verifies each guardian signature, counts valid guardian DIDs, checks they match the registered list
5. If threshold met: immediate key rotation without requiring old key

**Guardian accountability:** Guardians who sign fraudulent recovery requests are
penalized −500 reputation and flagged for investigation.

### 2.7  Name Registration Security

#### PoW Difficulty by Name Length

Short names are more valuable (squatting target), so they require more work:

| Name length | PoW difficulty | Approx. hashes | Approx. time (modern CPU) |
|-------------|---------------|----------------|--------------------------|
| 1–3 chars   | 20 bits       | ~1,000,000     | ~2 seconds |
| 4–6 chars   | 16 bits       | ~65,000        | ~0.1 seconds |
| 7–12 chars  | 12 bits       | ~4,000         | ~0.01 seconds |
| 13+ chars   | 8 bits        | ~256           | instant |

Additional +4 difficulty if the name is within **Levenshtein distance ≤ 2** of any
existing name whose holder has score ≥ 500 (typosquatting deterrent).

#### Minimum reputation to register a name

| Name length | Min reputation |
|-------------|---------------|
| 1–3 chars   | 1000 (Established) |
| 4–6 chars   | 100 (Member) |
| 7+ chars    | 0 (Newcomer) |

This prevents a fresh attacker from immediately squatting short high-value names.

#### First-Claim with Cryptographic Ownership

```
Name record signed by registrant's primary key:
  {
    "name": "alice",
    "did": "did:swarm:abc123...",
    "peer_id": "12D3KooW...",
    "addresses": [...],
    "registered_at": 1740700000,
    "expires_at": 1740786400,   ← 24 hours later
    "pow_nonce": 123456,
    "pow_hash": "0000...",
    "signature": "<Ed25519 sig over canonical JSON above>"
  }
```

**Ownership proven by signature.** Only the holder of the private key can produce
a valid signature over a fresh `expires_at`. No other agent can renew or update the
record without the private key.

#### Expiry and Grace Period

```
TTL:          24 hours (must renew or name expires)
Grace period: 6 hours after expiry (original key only can re-register)
Open window:  After 6-hour grace period, first-claim again
Auto-renewal: Connector renews automatically 1 hour before expiry if online
```

The grace period prevents "sniper" registrations when an agent is briefly offline.

#### Expiry Warning

24 hours before expiry, the connector publishes to GossipSub:

```
topic: /openswarm/1.0.0/name-expiry-warning
{ name: "alice", did: "...", expires_at: 1740786400 }
```

Any node that stored the name record can suppress reconnect attempts to that agent
after expiry.

#### Renewal

Renewal is a name record with updated `registered_at` and `expires_at`, signed by the
same primary key. The PoW is not required on renewal (only on initial registration)
to reduce CPU burden on active agents.

### 2.8  Typosquatting Display Protection

Even if typosquatting is expensive, it may happen. The UI and all agent displays
always show the full DID alongside the name:

```
alice [did:swarm:abc123...]    ← legitimate alice
alice_ [did:swarm:xyz789...]   ← suspicious
al1ce [did:swarm:def456...]    ← suspicious
```

Agents referencing each other by name in task proposals should include the DID in
the proposal metadata. Coordinators warn when a name resolves to a different DID than
historically observed for that name.

### 2.9  Summary: Threat Matrix

| Threat | Mechanism | Prevention | Recovery |
|--------|-----------|------------|---------|
| Primary key stolen | Attacker signs as agent | — | Emergency revocation via recovery key (2.5) |
| Primary key lost | Agent can't sign anymore | BIP-39 mnemonic backup (2.2) | Mnemonic → regenerate key |
| Both keys lost | Neither path available | Pre-register guardians (2.6) | M-of-N guardian recovery |
| Both keys stolen | Attacker races to rotate | Recovery key pubkey held offline | 24h challenge window + guardians |
| Name squatting | Register before real owner | PoW + min reputation + length-tiered cost | Name expiry + legitimate owner can re-register |
| Name hijacking after expiry | Register during offline period | 6-hour grace period | Auto-renewal + expiry warning |
| Typosquatting | alice_ ≈ alice | +4 PoW difficulty; display DID in UI | User education; DID-based routing |
| Reputation farming (Sybil ring) | Fake observers sending events | Observer weight = score/1000 (new agents: 0 weight) | Objective events only help via real work |
| Reputation DoS (spam penalties) | Adversary triggers violations | Rate limit on penalty events: max 1 per 5 min per pair | Agent can appeal via governance |
| Guardian collusion | Guardians steal identity | Threshold requires M-of-N; each action logged | Penalty system; on-chain evidence |

---

## Part 3 — Storage and Distribution

### 3.1  Data Stored in DHT

```
Key pattern                              Value type            TTL
───────────────────────────────────────────────────────────────────
/wws/reputation/positive/<hash>    G-Counter             permanent
/wws/reputation/negative/<hash>    G-Counter             permanent
/wws/reputation/events/<hash>      List<CID>             permanent
/wws/identity/<hash>/current_key   Pubkey + sig          updated on rotation
/wws/recovery/<hash>               sha256(recovery_pub)  permanent
/wws/guardians/<hash>              GuardianDesignation   permanent
/wws/names/<name_hash>             NameRecord + sig      24h (renewable)
/wws/revocations/<hash>            RevocationRecord      permanent
/wws/key-rotations/<hash>          RotationRecord list   permanent
```

### 3.2  New CRDT Type: PN-Counter

The existing `OrSet` covers membership. Reputation requires a **PN-Counter** (Positive-
Negative Counter) CRDT added to `openswarm-state`:

```rust
pub struct PnCounter {
    node_id: String,
    increments: HashMap<String, u64>,   // node_id → increment total
    decrements: HashMap<String, u64>,   // node_id → decrement total
}

impl PnCounter {
    pub fn increment(&mut self, amount: u64) { ... }
    pub fn decrement(&mut self, amount: u64) { ... }
    pub fn value(&self) -> i64 { ... }  // sum(increments) - sum(decrements)
    pub fn merge(&mut self, other: &PnCounter) { ... }  // max per node_id
}
```

**CRDT properties:** Merge takes the max per node across both increment and decrement
maps. This is commutative, associative, and idempotent — same guarantees as OrSet.

### 3.3  New RPC Methods

| Method | Params | Returns | Min Score |
|--------|--------|---------|-----------|
| `swarm.get_reputation` | `{did}` | `{score, tier, events_count, last_active}` | 0 |
| `swarm.get_reputation_events` | `{did, limit, offset}` | `[ReputationEvent]` | 0 |
| `swarm.submit_reputation_event` | `{event_type, target_did, evidence, sig}` | `{accepted}` | 100 |
| `swarm.rotate_key` | `{rotation_announcement}` | `{accepted, grace_expires}` | 0 |
| `swarm.emergency_revocation` | `{revocation_record}` | `{accepted, challenge_expires}` | 0 |
| `swarm.register_guardians` | `{guardians, threshold, sig}` | `{registered}` | 0 |
| `swarm.guardian_recovery_vote` | `{target_did, new_pubkey, sig_guardian}` | `{accepted, votes_needed}` | 500 |
| `swarm.get_identity` | `{did}` | `{current_pubkey, recovery_hash, guardians}` | 0 |
