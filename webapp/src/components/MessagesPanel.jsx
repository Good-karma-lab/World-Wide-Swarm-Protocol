import { useState, useEffect, useCallback } from 'react'
import { api } from '../api/client'

export default function MessagesPanel({ agents }) {
  const [messages, setMessages] = useState([])
  const [loading, setLoading] = useState(true)

  const fetchMessages = useCallback(async () => {
    try {
      const data = await api.inbox()
      setMessages(data.messages || [])
    } catch (_) {}
    setLoading(false)
  }, [])

  useEffect(() => {
    fetchMessages()
    const timer = setInterval(fetchMessages, 4000)
    return () => clearInterval(timer)
  }, [fetchMessages])

  const agentsList = agents?.agents || []
  const resolveName = (did) => {
    if (!did) return did
    const found = agentsList.find(a => a.agent_id === did)
    return found?.name || did.slice(-8)
  }

  if (loading) return <div style={{ color: 'var(--text-muted)', fontSize: 12 }}>Loading messages...</div>

  return (
    <div>
      <div className="detail-section-title">Agent Conversations ({messages.length})</div>
      <div className="log-box" style={{ maxHeight: '70vh' }}>
        {messages.length === 0 && <div style={{ color: 'var(--text-dim)' }}>No conversations yet.</div>}
        {messages.map((m, i) => {
          const isSent = m.direction === 'sent'
          const fromName = m.from_name || resolveName(m.from)
          const toName = m.to_name || resolveName(m.to)
          return (
            <div key={i} style={{
              marginBottom: 8,
              padding: '8px 12px',
              background: isSent ? 'rgba(0, 229, 176, 0.06)' : 'rgba(167, 139, 250, 0.06)',
              borderLeft: `3px solid ${isSent ? '#00e5b0' : '#a78bfa'}`,
              borderRadius: '0 6px 6px 0',
            }}>
              <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: 4 }}>
                <span style={{ fontSize: 11 }}>
                  <span style={{ color: isSent ? '#00e5b0' : '#a78bfa', fontWeight: 600 }}>
                    {fromName}
                  </span>
                  <span style={{ color: 'var(--text-muted)', margin: '0 6px' }}>→</span>
                  <span style={{ color: 'var(--text-dim)', fontWeight: 500 }}>
                    {toName}
                  </span>
                </span>
                <span style={{ fontSize: 10, color: 'var(--text-muted)' }}>
                  {m.timestamp ? new Date(m.timestamp).toLocaleTimeString() : ''}
                </span>
              </div>
              <div style={{ fontSize: 13, color: 'var(--text)', lineHeight: 1.5, whiteSpace: 'pre-wrap' }}>
                {m.content}
              </div>
            </div>
          )
        })}
      </div>
    </div>
  )
}
