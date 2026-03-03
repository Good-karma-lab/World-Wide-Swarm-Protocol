export default function Header({ agents, tasks, live, onSubmitClick, onAuditClick, onMessagesClick }) {
  const agentList = agents?.agents || []
  const taskList = tasks?.tasks || []

  const red    = agentList.filter(a => !a.connected).length
  const yellow = agentList.filter(a => a.connected && !a.loop_active).length
  const green  = agentList.filter(a => a.connected && a.loop_active).length

  return (
    <header className="header">
      <span className="header-brand" style={{
  fontFamily: "'Palatino Linotype', 'Book Antiqua', Palatino, serif",
  fontWeight: 300,
  letterSpacing: '0.25em',
  color: '#e8e8f0',
  textShadow: '0 0 20px rgba(232,232,240,0.8), 0 0 60px rgba(180,180,220,0.4)',
}}>
  WWS
</span>

      <div className="header-stats">
        <div className="header-stat">
          <span className="health-dot green" />
          <strong>{green}</strong> healthy
        </div>
        {yellow > 0 && (
          <div className="header-stat">
            <span className="health-dot yellow" />
            <strong>{yellow}</strong> degraded
          </div>
        )}
        {red > 0 && (
          <div className="header-stat">
            <span className="health-dot red" />
            <strong>{red}</strong> down
          </div>
        )}
        <div className="header-stat">
          <strong>{agentList.length}</strong> agents
        </div>
        <div className="header-stat">
          <strong>{taskList.length}</strong> tasks
        </div>
        {live?.active_tasks > 0 && (
          <div className="header-stat">
            <span className="health-dot green" style={{ animation: 'pulse 1.5s infinite' }} />
            <strong>{live.active_tasks}</strong> active
          </div>
        )}
      </div>

      <div className="header-actions">
        <button className="btn btn-ghost" onClick={onMessagesClick}>Messages</button>
        <button className="btn btn-ghost" onClick={onAuditClick}>Audit</button>
        <button className="btn btn-primary" onClick={onSubmitClick}>+ Submit Task</button>
      </div>
    </header>
  )
}
