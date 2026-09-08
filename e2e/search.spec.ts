import { expect, test } from '@playwright/test'
import { openInbox } from './helpers'

test.describe('search operators', () => {
  test('filters the inbox with is:unread and is:read', async ({ page }) => {
    await openInbox(page)
    await page.getByLabel('Search mail').fill('is:unread')
    await expect(page.getByText('Nora Li')).toBeVisible()
    await expect(page.getByText('Jonas Meier')).toBeVisible()
    await expect(page.getByText('Priya Shah')).toBeVisible()
    await page.getByLabel('Search mail').fill('is:read')
    await expect(page.getByText('No messages found')).toBeVisible()
  })

  test('scopes searches across folders with in:', async ({ page }) => {
    await openInbox(page)
    await page.getByLabel('Search mail').fill('in:all has:attachment')
    await expect(page.getByText('Jonas Meier')).toBeVisible()
    await expect(page.getByText('Alex Chen')).toBeVisible()
    await expect(page.getByText('Riley Brooks')).toBeVisible()
    await page.getByLabel('Search mail').fill('in:sent meridian')
    await expect(page.getByText('No messages found')).toBeVisible()
  })

  test('finds anything in the trash with a negated or scoped query', async ({ page }) => {
    await openInbox(page)
    await page.getByLabel('Select Design review notes').check()
    await page.getByLabel('Move to folder').selectOption('Trash')
    await page.getByLabel('Search mail').fill('in:trash priya')
    await expect(page.getByText('Design review notes')).toBeVisible()
    await expect(page.getByRole('heading', { name: 'Inbox', exact: true })).toBeVisible()
  })
})