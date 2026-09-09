import { expect, test } from '@playwright/test'
import { boot, inboxRow, openInbox } from './helpers'

test.describe('inbox', () => {
  test('lists seeded conversations', async ({ page }) => {
    await openInbox(page)
    await expect(page.getByText('Nora Li')).toBeVisible()
    await expect(page.getByText('Jonas Meier')).toBeVisible()
    await expect(page.getByText('Priya Shah')).toBeVisible()
    await expect(page.getByRole('heading', { name: 'Inbox', exact: true })).toBeVisible()
  })

  test('searches the mailbox from the topbar', async ({ page }) => {
    await openInbox(page)
    await page.getByLabel('Search mail').fill('meridian')
    await expect(page.getByText('Jonas Meier')).toBeVisible()
    await expect(page.getByText('Nora Li')).toBeHidden()
    await page.getByLabel('Search mail').fill('no-result-terminator')
    await expect(page.getByText('No messages found')).toBeVisible()
  })

  test('bulk selects and archives conversations', async ({ page }) => {
    await openInbox(page)
    await inboxRow(page, 'Q3 launch plan - review before Thursday').getByLabel('Select Q3 launch plan - review before Thursday').check()
    await page.locator('.sidebar').getByRole('button', { name: 'More', exact: true }).click()
    await page.getByRole('button', { name: 'Archive', exact: true }).click()
    await expect(page.getByText('Nora Li')).toBeHidden()
    await expect(page.getByRole('status').filter({ hasText: 'Archived' })).toBeVisible()
  })

  test('does not re-show onboarding on later pages', async ({ page }) => {
    await boot(page)
    await page.goto('/mail/sent')
    await expect(page.getByRole('heading', { name: 'Sent', exact: true })).toBeVisible()
    await expect(page.getByRole('dialog', { name: 'Welcome' })).toBeHidden()
  })
})
