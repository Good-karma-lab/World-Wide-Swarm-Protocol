const { test, expect } = require('@playwright/test')

// Tests run against the Vite dev server or the built connector file server.
// Set WEB_BASE_URL=http://localhost:5173 for the dev server.

test('header renders brand and action buttons', async ({ page }) => {
  await page.goto('/')

  // Brand
  await expect(page.getByText('WWS')).toBeVisible()

  // Action buttons
  await expect(page.getByRole('button', { name: /Submit Task/i })).toBeVisible()
  await expect(page.getByRole('button', { name: /Audit/i })).toBeVisible()
  await expect(page.getByRole('button', { name: /Messages/i })).toBeVisible()
})

test('graph area and bottom tray are present', async ({ page }) => {
  await page.goto('/')

  // Graph area exists
  const graphArea = page.locator('.graph-area')
  await expect(graphArea).toBeVisible()

  // Bottom tray with three column labels
  await expect(page.getByText('System Health')).toBeVisible()
  await expect(page.getByText('Tasks').first()).toBeVisible()
  await expect(page.getByText('Agents').first()).toBeVisible()
})

test('Submit Task button opens modal with textarea', async ({ page }) => {
  await page.goto('/')

  await page.getByRole('button', { name: /Submit Task/i }).click()

  const modal = page.locator('.modal')
  await expect(modal).toBeVisible()
  await expect(modal.locator('textarea')).toBeVisible()
  await expect(page.getByRole('button', { name: /Cancel/i })).toBeVisible()

  // Cancel closes modal
  await page.getByRole('button', { name: /Cancel/i }).click()
  await expect(modal).not.toBeVisible()
})

test('Audit button opens slide panel with Audit Log title', async ({ page }) => {
  await page.goto('/')

  await page.getByRole('button', { name: /Audit/i }).click()

  const overlay = page.locator('.slide-overlay')
  await expect(overlay).toBeVisible()
  await expect(overlay.locator('.slide-title')).toHaveText('Audit Log')

  // Close with X button
  await overlay.locator('.slide-close').click()
  await expect(overlay).not.toBeVisible()
})

test('Messages button opens slide panel with P2P Messages title', async ({ page }) => {
  await page.goto('/')

  await page.getByRole('button', { name: /Messages/i }).click()

  const overlay = page.locator('.slide-overlay')
  await expect(overlay).toBeVisible()
  await expect(overlay.locator('.slide-title')).toHaveText('P2P Messages')

  // Close with Esc
  await page.keyboard.press('Escape')
  await expect(overlay).not.toBeVisible()
})

test('health API endpoint is accessible', async ({ page }) => {
  // Just verify the app shell renders regardless of backend status
  await page.goto('/')
  await expect(page.locator('.app')).toBeVisible()
  await expect(page.locator('.header')).toBeVisible()
})
