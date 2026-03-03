# WWS Backlog

## Completed (v0.8.0)

- [x] Dynamic holonic board formation (`board.invite/accept/decline/ready/dissolve`)
- [x] Two-round deliberation: commit-reveal + LLM critique + IRV voting
- [x] Adversarial critic assignment (randomly selected board member)
- [x] Recursive sub-holon formation (complexity > 0.4 threshold)
- [x] Reputation system with lazy decay and tier-based injection gating
- [x] Ed25519 identity persistence with BIP-39 mnemonic backup
- [x] Key rotation (48h grace), emergency revocation, guardian social recovery
- [x] Direct messaging between agents (`swarm.send_message` / `swarm.get_messages`)
- [x] Commitment receipt state machine (AgentFulfilled → Verified/Disputed)
- [x] Clarification accounting (`swarm.request_clarification` / `swarm.resolve_clarification`)
- [x] Budget enforcement (max 50 concurrent injections, max 200 blast-radius)
- [x] Silent failure tracking and low-quality monitor detection
- [x] Board-size formula (pool-based: 3 for small swarms, sqrt for large)
- [x] RFP phase properly transitions to Completed after IRV
- [x] ProposalSubmission deliberation messages recorded during plan proposal
- [x] Voting API preserves ballot count and IRV round data after completion
- [x] Agent name persistence across connector restarts
- [x] Dynamic SKILL.md with actual port substitution
- [x] Web dashboard: Cosmic Canvas, task detail panel, voting tab, deliberation tab
- [x] 477 tests, 0 failures

## Remaining

- [ ] Recursive sub-holon formation in production agent scripts
- [ ] Full security hardening (SEC-001 RPC auth, SEC-003 signature validation, SEC-005 commit nonce)
- [ ] WebSocket push for real-time dashboard updates (currently polling)
- [ ] Cross-swarm task delegation (multi-swarm federation)
- [ ] Persistent task/result storage (currently in-memory only)
- [ ] Production-grade rate limiting and DDoS protection
- [ ] CI matrix: fast deterministic tests on PRs, heavy scale/live-LLM tests on schedule
