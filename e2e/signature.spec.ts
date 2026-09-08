import { expect, test, type Page } from '@playwright/test'

async function bootWithSignature(page: Page) {
  await page.addInitScript(() => {
    localStorage.setItem('harbor-mail:onboarded', '1')
    localStorage.setItem('harbor-mail:session', 'demo')
    localStorage.setItem('harbor-mail:settings', JSON.stringify({ signature: 'Chief Officer, Northstar Ops' }))
  })
  await page.goto('/mail/inbox')
  await expect(page.getByRole('heading', { name: 'Inbox', exact: true })).toBeVisible()
  await page.locator('.page-head').getByRole('button', { name: 'Compose' }).click()
  await expect(page.getByRole('dialog', { name: 'New message' })).toBeVisible()
}

test.describe('signature auto-append', () => {
  test('new messages start with the signature at the bottom and send it', async ({ page }) => {
    await bootWithSignature(page)
    const body = page.getByRole('textbox', { name: 'Message body' })
    await expect(body).toContainText('Chief Officer, Northstar Ops')
    await expect(page.getByLabel('Include signature')).toBeChecked()
    await page.getByPlaceholder('Recipients').fill('priya@harbor.co')
    await page.getByPlaceholder('Subject').fill('Signature check')
    await page.getByRole('textbox', { name: 'Message body' }).type('Hello from the desk.')
    await page.getByRole('button', { name: 'Send' }).click()
    await expect(page.getByRole('status').filter({ hasText: 'Message sent' })).toBeVisible()
    await expect.poll(() => page.evaluate(() => localStorage.getItem('harbor-mail:mailbox') ?? '')).toContain('Chief Officer, Northstar Ops')
  })

  test('per-message toggle removes and re-attaches the signature', async ({ page }) => {
    await bootWithSignature(page)
    const body = page.getByRole('textbox', { name: 'Message body' })
    await page.getByLabel('Include signature').uncheck()
    await expect(body).not.toContainText('Chief Officer, Northstar Ops')
    await page.getByLabel('Include signature').check()
    await expect(body).toContainText('Chief Officer, Northstar Ops')
  })
})