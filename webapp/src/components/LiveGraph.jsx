import { DataSet, Network } from 'vis-network/standalone'
import { useEffect, useRef, useState } from 'react'
import CosmicCanvas from './CosmicCanvas'

const HOLON_COLORS = {
  Forming:      '#636e72',
  Deliberating: '#ffaa00',
  Voting:       '#ff7675',
  Executing:    '#7c3aff',
  Synthesizing: '#a78bfa',
  Done:         '#00e5b0',
}

const ROLE_COLORS = {
  chair:    '#f59e0b',  // gold  — coordinator
  critic:   '#ff7675',  // coral — adversarial critic
  executor: '#00e5b0',  // teal  — subtask assigned
  member:   '#7c3aff',  // purple — participant
}

export default function LiveGraph({ topology, holons, agents, onNodeClick, taskHolon }) {
  const ref = useRef(null)
  const net = useRef(null)
  const [paused, setPaused] = useState(false)

  useEffect(() => {
    if (!ref.current) return
    if (!taskHolon) return  // cosmic canvas handles main swarm view

    const nodes = []
    const edges = []

    // ── Task-scoped holon view ──────────────────────────────────────────────
    if (taskHolon && taskHolon.task_id) {
      const holonNodeId = `holon:${taskHolon.task_id}`
      const holonColor = HOLON_COLORS[taskHolon.status] || '#636e72'

      // Build a fast lookup: agent_id → agent data
      const agentMap = {}
      ;(agents?.agents || []).forEach(a => { agentMap[a.agent_id] = a })

      // Build role map from members_detail if available, else derive from raw fields
      const roleMap = {}
      if (taskHolon.members_detail?.length) {
        taskHolon.members_detail.forEach(m => { roleMap[m.agent_id] = m.role })
      } else {
        const executorSet = new Set(Object.values(taskHolon.subtask_assignments || {}))
        ;(taskHolon.members || []).forEach(id => {
          if (id === taskHolon.chair) roleMap[id] = 'chair'
          else if (id === taskHolon.adversarial_critic) roleMap[id] = 'critic'
          else if (executorSet.has(id)) roleMap[id] = 'executor'
          else roleMap[id] = 'member'
        })
      }

      // Central holon node
      nodes.push({
        id: holonNodeId,
        label: `⬡ ${taskHolon.task_id.slice(0, 10)}…\n${taskHolon.status}  depth:${taskHolon.depth}`,
        color: { background: holonColor, border: holonColor, highlight: { background: '#fff', border: holonColor } },
        shape: 'diamond',
        size: 32,
        font: { color: '#fff', size: 11, bold: true, face: 'JetBrains Mono' },
        title: `Holon for task ${taskHolon.task_id}\nStatus: ${taskHolon.status}\nDepth: ${taskHolon.depth}\nMembers: ${(taskHolon.members || []).length}`,
      })

      // Member nodes
      ;(taskHolon.members || []).forEach(memberId => {
        const role = roleMap[memberId] || 'member'
        const color = ROLE_COLORS[role]
        const agentData = agentMap[memberId]
        const name = agentData?.name || (taskHolon.members_detail || []).find(m => m.agent_id === memberId)?.name || memberId.slice(-10)
        const isChair = role === 'chair'
        const isCritic = role === 'critic'

        nodes.push({
          id: memberId,
          label: name.length > 18 ? name.slice(0, 17) + '…' : name,
          color: { background: color, border: color, highlight: { background: '#fff', border: color } },
          shape: isChair ? 'box' : isCritic ? 'triangle' : 'dot',
          size: isChair ? 22 : isCritic ? 18 : 14,
          font: { color: '#fff', size: 10, face: 'JetBrains Mono' },
          title: `${name}\nRole: ${role}\nTasks done: ${agentData?.tasks_processed_count ?? '?'}\nReputation: ${agentData?.reputation_score ?? '?'}`,
        })

        edges.push({
          id: `member-${memberId}`,
          from: memberId,
          to: holonNodeId,
          color: { color, opacity: role === 'member' ? 0.45 : 0.75 },
          label: role !== 'member' ? role : '',
          font: { color, size: 9, face: 'JetBrains Mono', align: 'middle' },
          width: isChair ? 2.5 : isCritic ? 2 : 1,
          dashes: role === 'member',
          arrows: { to: { enabled: true, scaleFactor: 0.5 } },
        })
      })

      // Parent holon (if this is a sub-holon)
      if (taskHolon.parent_holon) {
        const parentId = `holon:${taskHolon.parent_holon}`
        nodes.push({
          id: parentId,
          label: `↑ ${taskHolon.parent_holon.slice(0, 10)}…\n(parent holon)`,
          color: { background: '#2a4a6a', border: '#3a7ab0' },
          shape: 'diamond',
          size: 20,
          font: { color: '#7ab0d8', size: 9, face: 'JetBrains Mono' },
          title: `Parent holon: ${taskHolon.parent_holon}`,
        })
        edges.push({
          id: 'parent-edge',
          from: parentId,
          to: holonNodeId,
          color: { color: '#3a7ab0', opacity: 0.6 },
          dashes: [8, 4],
          width: 1.5,
          arrows: { to: { enabled: true, scaleFactor: 0.6 } },
          label: 'spawned',
          font: { size: 9, color: '#5a90b8' },
        })
      }

      // Child holons (sub-holons for complex subtasks)
      ;(taskHolon.child_holons || []).forEach((childId, ci) => {
        const childHolon = (holons || []).find(h => h.task_id === childId)
        const childColor = childHolon ? (HOLON_COLORS[childHolon.status] || '#636e72') : '#3d1d7f'
        const childNodeId = `holon:${childId}`

        nodes.push({
          id: childNodeId,
          label: `⬡ ${childId.slice(0, 10)}…\n${childHolon?.status || 'sub-holon'}`,
          color: { background: childColor, border: childColor, highlight: { background: '#fff', border: childColor } },
          shape: 'diamond',
          size: 18,
          font: { color: '#fff', size: 9, face: 'JetBrains Mono' },
          title: `Sub-holon: ${childId}\nStatus: ${childHolon?.status || '?'}`,
        })
        edges.push({
          id: `child-${ci}`,
          from: holonNodeId,
          to: childNodeId,
          color: { color: '#a78bfa', opacity: 0.65 },
          dashes: [5, 3],
          width: 1.5,
          arrows: { to: { enabled: true, scaleFactor: 0.6 } },
          label: 'sub-task',
          font: { size: 9, color: '#a78bfa' },
        })
      })

      const options = {
        interaction: { hover: true, tooltipDelay: 200 },
        physics: {
          enabled: !paused,
          stabilization: { enabled: true, iterations: 200 },
          barnesHut: { springLength: 180, springConstant: 0.03, damping: 0.25, centralGravity: 0.4 },
        },
        edges: { smooth: { type: 'continuous' } },
        layout: { improvedLayout: true },
      }

      if (net.current) net.current.destroy()
      net.current = new Network(ref.current, { nodes: new DataSet(nodes), edges: new DataSet(edges) }, options)
      net.current.on('click', (params) => {
        if (params.nodes.length > 0 && onNodeClick) {
          const nodeId = params.nodes[0]
          if (nodeId.startsWith('holon:')) {
            const tid = nodeId.replace('holon:', '')
            const h = (holons || []).find(x => x.task_id === tid)
            if (h) onNodeClick({ type: 'holon', data: h })
          } else {
            const agent = (agents?.agents || []).find(a => a.agent_id === nodeId)
            if (agent) onNodeClick({ type: 'agent', data: { agent } })
          }
        }
      })
      return () => { if (net.current) net.current.destroy() }
    }
  }, [topology, holons, agents, taskHolon])

  useEffect(() => {
    if (net.current) {
      net.current.setOptions({ physics: { enabled: !paused } })
    }
  }, [paused])

  const fitGraph = () => { if (net.current) net.current.fit({ animation: true }) }

  return (
    <div className="graph-area">
      {/* Holon detail — vis-network */}
      {taskHolon && (
        <div id="live-graph" ref={ref} className="graph-container" />
      )}

      {/* Main swarm — cosmic canvas */}
      {!taskHolon && (
        <CosmicCanvas
          agents={agents}
          holons={holons}
          topology={topology}
          onNodeClick={onNodeClick}
        />
      )}

      {!taskHolon && (topology?.nodes || []).length === 0 && (holons || []).length === 0 && (
        <div className="graph-empty">
          Waiting for agents to connect…
        </div>
      )}

      <div className="graph-controls">
        {taskHolon ? (
          <>
            <span style={{ fontSize: 10, color: 'var(--platinum)', marginRight: 6, fontFamily: 'var(--font-mono)' }}>
              ⬡ Holon — {taskHolon.status}
            </span>
            {Object.entries(ROLE_COLORS).map(([role, color]) => (
              <span key={role} style={{ fontSize: 9, color, marginRight: 5 }}>■ {role}</span>
            ))}
            <button className="btn" style={{ fontSize: 11 }} onClick={fitGraph}>⊞ Fit</button>
            <button className="btn" style={{ fontSize: 11 }} onClick={() => setPaused(p => !p)}>
              {paused ? '▶ Resume' : '⏸ Pause'}
            </button>
          </>
        ) : null}
      </div>
    </div>
  )
}
