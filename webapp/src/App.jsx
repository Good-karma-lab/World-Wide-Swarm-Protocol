import { useCallback, useEffect, useState } from 'react'
import { api } from './api/client'
import { usePolling } from './hooks/usePolling'
import Header from './components/Header'
import LiveGraph from './components/LiveGraph'
import BottomTray from './components/BottomTray'
import SlidePanel from './components/SlidePanel'
import TaskDetailPanel from './components/TaskDetailPanel'
import AgentDetailPanel from './components/AgentDetailPanel'
import HolonDetailPanel from './components/HolonDetailPanel'
import AuditPanel from './components/AuditPanel'
import MessagesPanel from './components/MessagesPanel'
import SubmitTaskModal from './components/SubmitTaskModal'

export default function App() {
  // ── Data state ─────────────────────────
  const [voting, setVoting]             = useState({ voting: [], rfp: [] })
  const [messages, setMessages]         = useState([])
  const [tasks, setTasks]               = useState({ tasks: [] })
  const [agents, setAgents]             = useState({ agents: [] })
  const [topology, setTopology]         = useState({ nodes: [], edges: [] })
  const [audit, setAudit]               = useState({ events: [] })
  const [auth, setAuth]                 = useState({ token_required: false })
  const [holons, setHolons]             = useState([])
  const [conversations, setConversations] = useState([])
  const [live, setLive]                 = useState({ active_tasks: 0, known_agents: 0, messages: [], events: [] })

  // ── Task detail state ──────────────────
  const [taskId, setTaskId]             = useState('')
  const [taskTrace, setTaskTrace]       = useState({ timeline: [], descendants: [], messages: [] })
  const [taskVoting, setTaskVoting]     = useState({ voting: [], rfp: [] })
  const [taskBallots, setTaskBallots]   = useState({ ballots: [], irv_rounds: [] })
  const [taskHolon, setTaskHolon]       = useState(null) // holon detail for selected task

  // ── UI state ───────────────────────────
  const [panel, setPanel]               = useState(null) // { type, data }
  const [showSubmit, setShowSubmit]     = useState(false)
  const [description, setDescription]   = useState('')
  const [operatorToken, setOperatorToken] = useState(localStorage.getItem('openswarm.web.token') || '')
  const [submitError, setSubmitError]   = useState('')

  // ── Polling ────────────────────────────
  const refresh = useCallback(async () => {
    try {
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
      setVoting(v)
      setMessages(m)
      setTasks(t)
      setAgents(ag)
      setTopology(tp)
      setAudit(a)
      setAuth(au)
      setHolons(hl)
      setConversations(conv?.conversations ?? [])
    } catch (_) {
      // connector offline — keep existing state, retry next poll
    }
  }, [])

  usePolling(refresh, 5000)

  // ── WebSocket ──────────────────────────
  useEffect(() => {
    const proto = location.protocol === 'https:' ? 'wss' : 'ws'
    const ws = new WebSocket(`${proto}://${location.host}/api/stream`)
    ws.onmessage = (event) => {
      try {
        const payload = JSON.parse(event.data)
        if (payload.type === 'snapshot') setLive(payload)
      } catch (_) {}
    }
    return () => ws.close()
  }, [])

  // ── Task submission ────────────────────
  const submitTask = async () => {
    if (!description.trim()) return
    localStorage.setItem('openswarm.web.token', operatorToken || '')
    try {
      const res = await api.submitTask(description, operatorToken)
      setSubmitError('')
      setDescription('')
      setShowSubmit(false)
      if (res.task_id) loadTrace(res.task_id)
      await refresh()
    } catch (err) {
      setSubmitError(err.payload?.error || err.message)
    }
  }

  // ── Task trace loading ─────────────────
  const loadTrace = useCallback(async (requestedTaskId) => {
    const effectiveTaskId = (requestedTaskId || taskId || '').trim()
    if (!effectiveTaskId) return
    setTaskId(effectiveTaskId)
    const [trace, votingDetail, ballots, irvData, holon] = await Promise.all([
      api.taskTimeline(effectiveTaskId),
      api.votingTask(effectiveTaskId),
      api.taskBallots(effectiveTaskId).catch(() => ({ ballots: [] })),
      api.taskIrvRounds(effectiveTaskId).catch(() => ({ irv_rounds: [] })),
      api.holonDetail(effectiveTaskId).catch(() => null),
    ])
    setTaskTrace(trace)
    setTaskVoting(votingDetail)
    setTaskBallots({ ...ballots, irv_rounds: irvData.irv_rounds || [] })
    setTaskHolon(holon?.task_id ? holon : null)
  }, [taskId])

  // ── Panel helpers ──────────────────────
  const openTaskPanel = (task) => {
    loadTrace(task.task_id)
    setPanel({ type: 'task', data: { taskId: task.task_id } })
  }

  const openAgentPanel = (agent) => {
    setPanel({ type: 'agent', data: { agent } })
  }

  const openHolonPanel = (holon) => {
    setPanel({ type: 'holon', data: { holon } })
  }

  const handleGraphNodeClick = ({ type, data }) => {
    if (type === 'agent') openAgentPanel(data.agent)
    if (type === 'holon') openHolonPanel(data)
  }

  const closePanel = () => { setPanel(null); setTaskHolon(null) }

  // ── Render ─────────────────────────────
  return (
    <div className="app">
      <Header
        agents={agents}
        tasks={tasks}
        live={live}
        onSubmitClick={() => setShowSubmit(true)}
        onAuditClick={() => setPanel({ type: 'audit', data: {} })}
        onMessagesClick={() => setPanel({ type: 'messages', data: {} })}
      />

      <LiveGraph
        topology={topology}
        holons={holons}
        agents={agents}
        onNodeClick={handleGraphNodeClick}
        taskHolon={panel?.type === 'task' ? taskHolon : null}
      />

      <BottomTray
        agents={agents}
        tasks={tasks}
        onTaskClick={openTaskPanel}
        onAgentClick={openAgentPanel}
      />

      {panel?.type === 'task' && (
        <SlidePanel
          title={`Task: ${(panel.data.taskId || '').slice(0, 16)}…`}
          onClose={closePanel}
        >
          <TaskDetailPanel
            taskId={panel.data.taskId}
            taskTrace={taskTrace}
            taskVoting={taskVoting}
            taskBallots={taskBallots}
            agents={agents}
            onTaskClick={openTaskPanel}
          />
        </SlidePanel>
      )}

      {panel?.type === 'agent' && (
        <SlidePanel
          title={`Agent: ${(panel.data.agent?.name || panel.data.agent?.agent_id || '').slice(0, 24)}`}
          onClose={closePanel}
        >
          <AgentDetailPanel
            agent={panel.data.agent}
            tasks={tasks}
            onTaskClick={openTaskPanel}
          />
        </SlidePanel>
      )}

      {panel?.type === 'holon' && (
        <SlidePanel
          title={`Holon: ${(panel.data.holon?.task_id || '').slice(0, 16)}…`}
          onClose={closePanel}
        >
          <HolonDetailPanel
            holon={panel.data.holon}
            holons={holons}
            onTaskClick={openTaskPanel}
            onHolonClick={openHolonPanel}
          />
        </SlidePanel>
      )}

      {panel?.type === 'audit' && (
        <SlidePanel title="Audit Log" onClose={closePanel}>
          <AuditPanel audit={audit} />
        </SlidePanel>
      )}

      {panel?.type === 'messages' && (
        <SlidePanel title="P2P Messages" onClose={closePanel}>
          <MessagesPanel conversations={conversations} agents={agents?.agents || []} />
        </SlidePanel>
      )}

      {showSubmit && (
        <SubmitTaskModal
          description={description}
          setDescription={setDescription}
          operatorToken={operatorToken}
          setOperatorToken={setOperatorToken}
          auth={auth}
          onSubmit={submitTask}
          onClose={() => { setShowSubmit(false); setSubmitError('') }}
          submitError={submitError}
        />
      )}
    </div>
  )
}
