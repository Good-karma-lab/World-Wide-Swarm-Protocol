function scrub(s) {
  return String(s || '').replace(/did:swarm:[A-Za-z0-9]+/g, m => '[' + m.slice(-6) + ']')
}

function healthLabel(a) {
  if (!a.connected) return { text: 'DOWN', cls: 'badge-coral' }
  if (!a.loop_active) return { text: 'DEGRADED', cls: 'badge-amber' }
  return { text: 'HEALTHY', cls: 'badge-platinum' }
}

function mini(label, value, color) {
  return (
    <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: 2 }}>
      <span style={{ fontSize: 11, color: 'var(--text-muted)' }}>{label}</span>
      <span style={{ fontSize: 11, fontFamily: 'var(--font-mono)', color: color || 'var(--text)' }}>{value}</span>
    </div>
  )
}

function ReputationBar({ agent }) {
  const s = agent.reputation_score || 0

  // Three signal contributions (approximate; backend has full formula)
  const done = agent.tasks_processed_count || 0
  const got  = agent.tasks_assigned_count  || 0
  const rel  = got > 0 ? done / (got + 1) : 0
  const sigR = done * rel * 0.10
  const sigD = ((agent.plans_proposed_count || 0) + (agent.votes_cast_count || 0)) * 0.02
  const sigI = Math.sqrt(agent.tasks_injected_count || 0) * 0.05
  const total = sigR + sigD + sigI || 0.001

  const color = s >= 0.5 ? 'var(--teal)' : s > 0 ? '#ffaa00' : 'var(--coral)'
  // Bar anchored at inject gate (0.5 = 100%); can overflow
  const barPct = Math.min(100, (s / 0.5) * 100)

  // Signal bar widths proportional to contribution
  const wR = (sigR / total) * barPct
  const wD = (sigD / total) * barPct
  const wI = (sigI / total) * barPct

  return (
    <div style={{ marginBottom: 12 }}>
      <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: 6 }}>
        <span style={{ fontSize: 11, color: 'var(--text-muted)' }}>Trust score</span>
        <span style={{ fontSize: 13, fontWeight: 700, color, fontFamily: 'var(--font-mono)' }}>
          {s.toFixed(2)}
        </span>
      </div>

      {/* Segmented bar: reliability | deliberation | ecosystem */}
      <div style={{ background: 'var(--border)', borderRadius: 4, height: 7, overflow: 'hidden', display: 'flex', marginBottom: 6 }}>
        <div style={{ width: `${wR}%`, background: 'var(--teal)',    height: '100%', transition: 'width 0.4s ease' }} title="Reliability" />
        <div style={{ width: `${wD}%`, background: '#a78bfa',        height: '100%', transition: 'width 0.4s ease' }} title="Deliberation" />
        <div style={{ width: `${wI}%`, background: '#f59e0b',        height: '100%', transition: 'width 0.4s ease' }} title="Ecosystem" />
      </div>

      {/* Signal breakdown */}
      <div style={{ display: 'flex', gap: 10, marginBottom: 6 }}>
        <span style={{ fontSize: 10, color: 'var(--teal)' }}>■ Reliability {sigR.toFixed(2)}</span>
        <span style={{ fontSize: 10, color: '#a78bfa' }}>■ Deliberation {sigD.toFixed(2)}</span>
        <span style={{ fontSize: 10, color: '#f59e0b' }}>■ Ecosystem {sigI.toFixed(2)}</span>
      </div>

      {/* Formula explanation */}
      <div style={{ fontSize: 10, color: 'var(--text-muted)', lineHeight: 1.7 }}>
        <strong style={{ color: 'var(--teal)' }}>Reliability</strong> = done² / (assigned+1) × 0.10 — rewards finishing what you take<br />
        <strong style={{ color: '#a78bfa' }}>Deliberation</strong> = (plans×honesty + votes) × 0.02 — rewards honest participation<br />
        <strong style={{ color: '#f59e0b' }}>Ecosystem</strong> = √injected × 0.05 — rewards contributing new work<br />
        Inject rights unlock at 5 completed tasks.
      </div>
    </div>
  )
}

export default function AgentDetailPanel({ agent, tasks, onTaskClick }) {
  if (!agent) return null
  const health = healthLabel(agent)
  const taskList = (tasks?.tasks || []).filter(t =>
    t.assigned_to === agent.agent_id || t.assigned_to_name === agent.name
  )

  return (
    <div>
      {/* Header meta */}
      <div className="detail-meta" style={{ marginBottom: 16 }}>
        <span>ID: <strong>{scrub(agent.agent_id)}</strong></span>
        <span>Name: <strong>{scrub(agent.name)}</strong></span>
        <span>Tier: <strong>{agent.tier}</strong></span>
        <span className={`badge ${health.cls}`}>{health.text}</span>
        {agent.can_inject_tasks
          ? <span className="badge badge-platinum" title="Can submit tasks to the swarm">✓ Can inject tasks</span>
          : <span className="badge badge-amber" title="Must complete at least 5 tasks first">⚠ No inject rights</span>
        }
      </div>

      {/* Reputation */}
      <div className="detail-section">
        <div className="detail-section-title">Reputation</div>
        <ReputationBar agent={agent} />
      </div>

      {/* Stats */}
      <div className="detail-section">
        <div className="detail-section-title">Activity</div>
        <table className="data-table">
          <thead>
            <tr>
              <th>Metric</th>
              <th>Value</th>
            </tr>
          </thead>
          <tbody>
            <tr><td>Connected</td><td>{agent.connected ? 'yes' : 'no'}</td></tr>
            <tr><td>Loop active</td><td>{agent.loop_active ? 'yes' : 'no'}</td></tr>
            <tr><td>Tasks assigned</td><td>{agent.tasks_assigned_count ?? 0}</td></tr>
            <tr><td>Tasks processed</td><td>{agent.tasks_processed_count ?? 0}</td></tr>
            <tr><td>Plans proposed</td><td>{agent.plans_proposed_count ?? 0}</td></tr>
            <tr><td>Plans revealed</td><td>{agent.plans_revealed_count ?? 0}</td></tr>
            <tr><td>Votes cast</td><td>{agent.votes_cast_count ?? 0}</td></tr>
            <tr><td>Tasks injected</td><td>{agent.tasks_injected_count ?? 0}</td></tr>
            <tr><td>Last poll (s)</td><td>{agent.last_task_poll_secs ?? '—'}</td></tr>
            <tr><td>Last result (s)</td><td>{agent.last_result_secs ?? '—'}</td></tr>
          </tbody>
        </table>
      </div>

      {/* Assigned tasks */}
      {taskList.length > 0 && (
        <div className="detail-section">
          <div className="detail-section-title">Assigned Tasks</div>
          {taskList.map(t => (
            <div
              key={t.task_id}
              onClick={() => onTaskClick && onTaskClick(t)}
              style={{
                padding: '6px 10px',
                background: 'var(--surface-2)',
                border: '1px solid var(--border)',
                borderRadius: 5,
                marginBottom: 4,
                cursor: 'pointer',
                fontFamily: 'var(--font-mono)',
                fontSize: 11,
              }}
            >
              <span style={{ color: 'var(--teal)' }}>{(t.task_id || '').slice(0, 12)}…</span>
              {' '}
              <span style={{ color: 'var(--text-muted)' }}>{t.status}</span>
              {' '}
              <span>{t.description?.slice(0, 60) || ''}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  )
}
