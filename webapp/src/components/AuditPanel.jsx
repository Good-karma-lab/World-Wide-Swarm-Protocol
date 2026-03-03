function scrub(s) {
  return String(s || '').replace(/did:swarm:[A-Za-z0-9]+/g, m => '[' + m.slice(-6) + ']')
}

export default function AuditPanel({ audit }) {
  const events = audit?.events || []
  return (
    <div>
      <div className="detail-section-title">Operator Audit Log</div>
      <div className="log-box" style={{ maxHeight: '70vh' }}>
        {events.length === 0 && <div style={{ color: 'var(--text-dim)' }}>No audit events yet.</div>}
        {events.map((e, i) => (
          <div key={i} style={{ marginBottom: 2 }}>
            <span style={{ color: 'var(--text-muted)' }}>[{e.timestamp}]</span>{' '}
            {scrub(e.message)}
          </div>
        ))}
      </div>
    </div>
  )
}
