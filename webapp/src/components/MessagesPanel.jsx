function scrubId(s) {
  return String(s || '').replace(/did:swarm:[A-Za-z0-9_-]+/g, m => m.slice(-8))
}

function resolveName(did, agents) {
  if (!did) return '?'
  const agent = (agents || []).find(a => a.agent_id === did)
  return agent?.name || scrubId(did)
}

export default function MessagesPanel({ conversations, agents }) {
  if (!conversations || !conversations.length) {
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
                  {resolveName(c.from, agents)}
                </span>
                <span style={{ color: 'var(--text-muted)' }}>→</span>
                <span style={{ color: 'var(--text)' }}>
                  {resolveName(c.to, agents)}
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
