import { expect, test } from '@playwright/test'
import { openInbox, openThread } from './helpers'

const composeButton = (page: import('@playwright/test').Page) => page.locator('.page-head').getByRole('button', { name: 'Compose' })

async function sendComposed(page: import('@playwright/test').Page, recipients: string, subject: string, body: string) {
  await composeButton(page).click()
  const composer = page.getByRole('dialog', { name: 'New message' })
  await expect(composer).toBeVisible()
  await page.getByPlaceholder('Recipients').fill(recipients)
  await page.getByPlaceholder('Subject').fill(subject)
  await page.getByRole('textbox', { name: 'Message body' }).fill(body)
  await page.getByRole('button', { name: 'Send' }).click()
  await expect(page.getByRole('status').filter({ hasText: 'Message sent' })).toBeVisible()
}

test.describe('delivery realism', () => {
  test('counts down before an undo-send window closes and unsends', async ({ page }) => {
    await openInbox(page)
    await sendComposed(page, 'riley@papertrail.com', 'Undo check', 'Please ignore this one.')
    const undoButton = page.getByRole('button', { name: /^Undo · \d+s$/ })
    await expect(undoButton).toBeVisible({ timeout: 2500 })
    await expect(page.getByRole('button', { name: /^Undo · \d+s$/ })).toHaveCount(1)
    await undoButton.click()
    await expect.poll(() => page.evaluate(() => localStorage.getItem('harbor-mail:mailbox') ?? '')).not.toContain('Undo check')
    await page.goto('/mail/sent')
    await expect(page.getByRole('heading', { name: 'Sent', exact: true })).toBeVisible()
    await expect(page.getByText('Undo check')).toHaveCount(0)
  })

  test('sends a read receipt when the sender requests one', async ({ page }) => {
    await openInbox(page)
    await page.goto('/mail/settings')
    const settings = page.getByRole('dialog', { name: 'Settings' })
    await expect(settings).toBeVisible()
    await settings.getByLabel('Send read receipts by default').check()
    await openThread(page, 'Nora Li')
    await expect(page.getByRole('status').filter({ hasText: 'Read receipt sent to Nora Li' })).toBeVisible()
  })

  test('delivers a reply with a desktop notification and unread badge', async ({ page }) => {
    await page.addInitScript(() => {
      window.__deliveryNotifications = []
      class DeskNotif {
        static permission = 'granted'
        title: string
        body?: string
        onClick: (() => void) | null = null
        set onclick(handler: () => void) { this.onClick = handler }
        constructor(title: string, options?: { body?: string; tag?: string }) {
          this.title = title
          this.body = options?.body
          window.__deliveryNotifications.push({ title, body: options?.body ?? '' })
        }
      }
      Object.defineProperty(window, 'Notification', { value: DeskNotif, configurable: true })
    })
    await openInbox(page)
    await sendComposed(page, 'nora@northstar.studio', 'Delivery test', 'Hello from the prototype.')
    await expect(page.getByRole('status').filter({ hasText: 'New mail from Nora Li' })).toBeVisible({ timeout: 15000 })
    await expect(page).toHaveTitle(/\(\d+\) Harbor Mail/)
    await expect.poll(() => page.evaluate(() => window.__deliveryNotifications?.map(item => item.body) ?? []).then(bodies => bodies.join(' '))).toContain('Delivery test')
  })
})