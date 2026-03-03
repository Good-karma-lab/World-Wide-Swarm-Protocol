function scrub(s) {
  return String(s || '').replace(/did:swarm:[A-Za-z0-9]+/g, m => '[' + m.slice(-6) + ']')
}

const STATUS_BADGE = {
  Forming:      'badge-dim',
  Deliberating: 'badge-amber',
  Voting:       'badge-coral',
  Executing:    'badge-violet',
  Synthesizing: 'badge-violet',
  Done:         'badge-platinum',
}

export default function HolonDetailPanel({ holon, holons, onTaskClick, onHolonClick }) {
  if (!holon) return null
  const allHolons = holons || []
  const children = allHolons.filter(h => h.parent_holon === holon.task_id)
  const badgeCls = STATUS_BADGE[holon.status] || 'badge-dim'

  return (
    <div>
      {/* Header meta */}
      <div className="detail-meta" style={{ marginBottom: 20 }}>
        <span>Task: <strong style={{ fontFamily: 'var(--font-mono)' }}>{(holon.task_id || '').slice(0, 20)}…</strong></span>
        <span className={`badge ${badgeCls}`}>{holon.status}</span>
        <span className="badge badge-dim">Depth {holon.depth}</span>
      </div>

      {/* Composition */}
      <div className="detail-section">
        <div className="detail-section-title">Composition</div>
        <div className="detail-meta">
          <span>Chair: <strong style={{ color: 'var(--teal)' }}>{scrub(holon.chair)}</strong></span>
          {holon.adversarial_critic && (
            <span>Critic: <strong style={{ color: 'var(--coral)' }}>{scrub(holon.adversarial_critic)}</strong></span>
          )}
          <span>Members: <strong>{holon.members?.length || 0}</strong></span>
          {holon.parent_holon && (
            <span>Parent: <strong style={{ fontFamily: 'var(--font-mono)', fontSize: 10 }}>{holon.parent_holon.slice(0, 16)}…</strong></span>
          )}
        </div>
      </div>

      {/* Member list */}
      {holon.members?.length > 0 && (
        <div className="detail-section">
          <div className="detail-section-title">Members</div>
          <div style={{ display: 'flex', flexWrap: 'wrap', gap: 6 }}>
            {holon.members.map((m, i) => (
              <span key={i} className="member-chip">{scrub(m)}</span>
            ))}
          </div>
        </div>
      )}

      {/* Child holons */}
      {children.length > 0 && (
        <div className="detail-section">
          <div className="detail-section-title">Child Holons ({children.length})</div>
          {children.map(child => (
            <div
              key={child.task_id}
              onClick={() => onHolonClick && onHolonClick(child)}
              style={{
                padding: '6px 10px',
                background: 'var(--surface-2)',
                border: '1px solid var(--border)',
                borderRadius: 5,
                marginBottom: 4,
                cursor: 'pointer',
                display: 'flex',
                gap: 10,
                alignItems: 'center',
                fontSize: 11,
                fontFamily: 'var(--font-mono)',
              }}
            >
              <span className={`badge ${STATUS_BADGE[child.status] || 'badge-dim'}`} style={{ fontSize: 10 }}>{child.status}</span>
              <span style={{ color: 'var(--teal)' }}>{(child.task_id || '').slice(0, 14)}…</span>
              <span style={{ color: 'var(--text-muted)' }}>d{child.depth}</span>
            </div>
          ))}
        </div>
      )}

      {/* Link to task */}
      <div style={{ display: 'flex', gap: 8 }}>
        <button
          className="btn"
          onClick={() => onTaskClick && onTaskClick({ task_id: holon.task_id })}
        >
          View Task Detail →
        </button>
      </div>
    </div>
  )
}
