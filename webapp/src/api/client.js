export async function fetchJson(url, options) {
  const res = await fetch(url, options)
  const ct = res.headers.get('content-type') || ''
  if (!ct.includes('application/json')) {
    const err = new Error(`HTTP ${res.status}`)
    err.status = res.status
    throw err
  }
  const data = await res.json()
  if (!res.ok) {
    const err = new Error(data.error || 'request_failed')
    err.status = res.status
    err.payload = data
    throw err
  }
  return data
}

export const api = {
  health: () => fetchJson('/api/health'),
  authStatus: () => fetchJson('/api/auth-status'),
  hierarchy: () => fetchJson('/api/hierarchy'),
  voting: () => fetchJson('/api/voting'),
  votingTask: (taskId) => fetchJson(`/api/voting/${taskId}`),
  messages: () => fetchJson('/api/messages'),
  tasks: () => fetchJson('/api/tasks'),
  agents: () => fetchJson('/api/agents'),
  flow: () => fetchJson('/api/flow'),
  topology: () => fetchJson('/api/topology'),
  recommendations: () => fetchJson('/api/ui-recommendations'),
  audit: () => fetchJson('/api/audit'),
  taskTimeline: (taskId) => fetchJson(`/api/tasks/${taskId}/timeline`),
  submitTask: (description, token) =>
    fetchJson('/api/tasks', {
      method: 'POST',
      headers: {
        'content-type': 'application/json',
        'x-ops-token': token || ''
      },
      body: JSON.stringify({ description })
    }),
  holons: () => fetchJson('/api/holons').then(d => d.holons || []),
  holonDetail: (taskId) => fetchJson(`/api/holons/${taskId}`),
  taskDeliberation: (taskId) => fetchJson(`/api/tasks/${taskId}/deliberation`),
  taskBallots: (taskId) => fetchJson(`/api/tasks/${taskId}/ballots`),
  taskIrvRounds: (taskId) => fetchJson(`/api/tasks/${taskId}/irv-rounds`),
}
