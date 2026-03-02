const { test, expect } = require('@playwright/test')

async function waitFor(check, timeoutMs = 60000, stepMs = 1000) {
  const start = Date.now()
  for (;;) {
    const result = await check()
    if (result) return result
    if (Date.now() - start > timeoutMs) {
      throw new Error('Condition not met before timeout')
    }
    await new Promise((r) => setTimeout(r, stepMs))
  }
}

test('swarm web console shows requested capabilities', async ({ page }) => {
  test.setTimeout(300000)
  page.on('crash', () => {
    throw new Error('Browser page crashed during swarm web E2E')
  })

  // 1) Load the app — brand + layout
  await page.goto('/')
  await expect(page.getByText('WWS')).toBeVisible()
  await expect(page.locator('.graph-area')).toBeVisible()
  await expect(page.getByText('System Health')).toBeVisible()
  await expect(page.getByText('Tasks').first()).toBeVisible()
  await expect(page.getByText('Agents').first()).toBeVisible()

  // 2) Wait for at least one agent to register
  const agentsData = await waitFor(async () => {
    const resp = await page.request.get('/api/agents')
    const payload = await resp.json()
    return (payload.agents?.length || 0) > 0 ? payload : null
  }, 120000)
  expect(agentsData.agents.length).toBeGreaterThan(0)

  // 3) Submit a task via the web UI modal
  const taskText = `Playwright e2e task ${Date.now()}`
  await page.getByRole('button', { name: /Submit Task/i }).click()
  const modal = page.locator('.modal')
  await expect(modal).toBeVisible()
  await modal.locator('textarea').fill(taskText)
  await modal.getByRole('button', { name: /^Submit$/i }).click()
  // Modal closes after successful submit
  await expect(modal).not.toBeVisible({ timeout: 15000 })

  // 4) Verify task appears in /api/tasks
  const tasksResp = await page.request.get('/api/tasks')
  const tasksPayload = await tasksResp.json()
  const submitted = (tasksPayload.tasks || []).find((t) => (t.description || '').includes(taskText))
  expect(submitted).toBeTruthy()

  // 5) Messages slide panel
  await page.getByRole('button', { name: /Messages/i }).click()
  const overlay = page.locator('.slide-overlay')
  await expect(overlay).toBeVisible()
  await expect(overlay.locator('.slide-title')).toHaveText('P2P Messages')
  await expect(page.getByText('P2P Business Messages')).toBeVisible()
  // Close
  await overlay.locator('.slide-close').click()
  await expect(overlay).not.toBeVisible()

  // 6) Audit slide panel — should show AUDIT task.inject after submit
  await page.getByRole('button', { name: /Audit/i }).click()
  await expect(overlay).toBeVisible()
  await expect(overlay.locator('.slide-title')).toHaveText('Audit Log')
  await expect(page.getByText('Operator Audit Log')).toBeVisible()
  await expect(page.locator('.log-box')).toContainText('AUDIT task.inject', { timeout: 15000 })
  // Close
  await overlay.locator('.slide-close').click()
  await expect(overlay).not.toBeVisible()

  // 7) Verify core API endpoints return valid structures
  const topologyResp = await page.request.get('/api/topology')
  const topology = await topologyResp.json()
  expect(Array.isArray(topology.nodes)).toBeTruthy()

  const votingResp = await page.request.get('/api/voting')
  const votingPayload = await votingResp.json()
  expect(Array.isArray(votingPayload.rfp)).toBeTruthy()
  expect(Array.isArray(votingPayload.voting)).toBeTruthy()

  const messagesResp = await page.request.get('/api/messages')
  const messagesPayload = await messagesResp.json()
  expect(Array.isArray(messagesPayload)).toBeTruthy()

  // 8) Click a task in the bottom tray to open task detail panel
  const taskItem = page.locator('.task-item').first()
  await expect(taskItem).toBeVisible({ timeout: 10000 })
  await taskItem.click()
  await expect(overlay).toBeVisible()
  await expect(overlay.locator('.slide-title')).toContainText('Task:')
  await overlay.locator('.slide-close').click()
})
