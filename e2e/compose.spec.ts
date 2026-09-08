import { expect, test } from '@playwright/test'
import { openInbox } from './helpers'

const composeButton = (page: import('@playwright/test').Page) => page.locator('.page-head').getByRole('button', { name: 'Compose' })

test.describe('compose', () => {
  test('composes and sends a message that lands in Sent', async ({ page }) => {
    await openInbox(page)
    await composeButton(page).click()
    const composer = page.getByRole('dialog', { name: 'New message' })
    await expect(composer).toBeVisible()
    await page.getByPlaceholder('Recipients').fill('priya@harbor.co')
    await page.getByPlaceholder('Subject').fill('Lunch next week')
    await page.getByRole('textbox', { name: 'Message body' }).fill('See you at twelve.')
    await page.getByRole('button', { name: 'Send' }).click()
    await expect(page.getByRole('status').filter({ hasText: 'Message sent' })).toBeVisible()
    await expect(composer).toBeHidden()
    await expect.poll(() => page.evaluate(() => localStorage.getItem('harbor-mail:mailbox') ?? '')).toContain('Lunch next week')
    await page.goto('/mail/sent')
    await expect(page.getByRole('heading', { name: 'Sent', exact: true })).toBeVisible()
    await expect(page.getByText('Lunch next week')).toBeVisible()
  })

  test('opens compose from the keyboard', async ({ page }) => {
    await openInbox(page)
    await page.getByRole('heading', { name: 'Inbox', exact: true }).click()
    await page.keyboard.press('c')
    await expect(page.getByRole('dialog', { name: 'New message' })).toBeVisible()
  })

  test('keeps an autosaved draft for the next session', async ({ page }) => {
    await openInbox(page)
    await composeButton(page).click()
    await page.getByPlaceholder('Subject').fill('Autosave check')
    await expect.poll(() => page.evaluate(() => localStorage.getItem('harbor-mail:compose-draft') ?? '').then(value => value.includes('Autosave check'))).toBe(true)
    await page.reload()
    await composeButton(page).click()
    await expect(page.getByPlaceholder('Subject')).toHaveValue('Autosave check')
  })
})