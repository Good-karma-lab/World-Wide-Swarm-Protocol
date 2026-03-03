const SKIP_METHODS = new Set(['keepalive', 'ping', 'pong', 'peer_discovery', 'peer_announce', 'swarm_join', 'swarm_leave'])
const SKIP_TOPICS  = new Set(['_keepalive', '_discovery', '_internal'])

function scrub(s) {
  return String(s || '').replace(/did:swarm:[A-Za-z0-9]+/g, m => '[' + m.slice(-6) + ']')
}

export default function MessagesPanel({ messages }) {
  const filtered = (messages || []).filter(m => {
    if (SKIP_METHODS.has((m.method || '').toLowerCase())) return false
    if (SKIP_TOPICS.has((m.topic || '').toLowerCase()))   return false
    return true
  })

  return (
    <div>
      <div className="detail-section-title">P2P Business Messages ({filtered.length})</div>
      <div className="log-box" style={{ maxHeight: '70vh' }}>
        {filtered.length === 0 && <div style={{ color: 'var(--text-dim)' }}>No messages yet.</div>}
        {filtered.map((m, i) => (
          <div key={i} style={{ marginBottom: 2 }}>
            <span style={{ color: 'var(--text-muted)' }}>[{m.timestamp}]</span>{' '}
            <span style={{ color: 'var(--platinum, #e8e8f0)' }}>{m.topic}</span>{' '}
            {m.method && <span style={{ color: '#a78bfa' }}>{m.method}</span>}{' '}
            {scrub(m.outcome || '')}
          </div>
        ))}
      </div>
    </div>
  )
}
