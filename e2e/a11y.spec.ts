import AxeBuilder from '@axe-core/playwright'
import { expect, test } from '@playwright/test'
import { boot, openInbox } from './helpers'

const scan = async (page: import('@playwright/test').Page) => {
  const results = await new AxeBuilder({ page }).analyze()
  const summary = results.violations
    .map(v => `${v.id} (${v.impact}) — ${v.help}\n  ${v.nodes.map(n => `targets=${n.target.join(' ')} :: ${n.failureSummary}`).join('\n  ')}`)
    .join('\n\n')
  expect.soft(results.violations, summary).toEqual([])
}

test('login page has no accessibility violations', async ({ page }) => {
  await boot(page)
  await page.goto('/login')
  await expect(page.getByText('Sign in to your mailbox')).toBeVisible()
  await scan(page)
})

test('inbox has no accessibility violations', async ({ page }) => {
  await openInbox(page)
  await scan(page)
})

test('thread view has no accessibility violations', async ({ page }) => {
  await openInbox(page)
  await page.locator('.mail-row', { hasText: 'Nora Li' }).getByRole('button', { name: /^Open / }).click()
  await expect(page).toHaveURL(/\/thread\//)
  await expect(page.locator('.reader')).toBeVisible()
  await scan(page)
})

test('composer has no accessibility violations', async ({ page }) => {
  await openInbox(page)
  await page.locator('.page-head').getByRole('button', { name: 'Compose' }).click()
  await expect(page.getByRole('dialog', { name: 'New message' })).toBeVisible()
  await scan(page)
})