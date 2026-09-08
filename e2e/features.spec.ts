import { expect, test, type Page } from '@playwright/test'
import { openInbox } from './helpers'

test.describe('custom folders', () => {
  test('creates a folder and moves mail into it', async ({ page }) => {
    await openInbox(page)

    await page.getByRole('button', { name: 'Manage folders' }).click()
    const dialog = page.getByRole('dialog', { name: 'Manage folders' })
    await expect(dialog).toBeVisible()
    await dialog.getByLabel('New folder name').fill('Legal')
    await dialog.getByRole('button', { name: 'Add', exact: true }).click()
    await expect(dialog.getByLabel('Rename Legal')).toBeVisible()
    await dialog.getByRole('button', { name: 'Done', exact: true }).click()

    await page.locator('.sidebar').getByRole('button', { name: /^Legal/ }).click()
    await expect(page.getByRole('heading', { name: 'Legal', exact: true })).toBeVisible()
    await expect(page.getByText('Nothing here yet')).toBeVisible()

    await page.locator('.sidebar').getByRole('button', { name: /^Inbox/ }).click()
    await page.getByLabel('Select Design review notes').check()
    await page.getByLabel('Move to folder').selectOption('Legal')
    await page.locator('.sidebar').getByRole('button', { name: /^Legal/ }).click()
    await expect(page.getByText('Design review notes')).toBeVisible()
  })
})

test.describe('settings — mail', () => {
  async function openSettings(page: Page) {
    await openInbox(page)
    await page.getByRole('button', { name: /Alex Morgan/ }).click()
    await page.getByRole('menuitem', { name: 'Settings', exact: true }).click()
    const settings = page.getByRole('dialog', { name: 'Settings' })
    await expect(settings).toBeVisible()
    return settings
  }

  test('auto-reply and forwarding', async ({ page }) => {
    const settings = await openSettings(page)
    await settings.getByRole('button', { name: 'Mail', exact: true }).click()

    await page.getByRole('checkbox', { name: /Turn on automatic replies/ }).check()
    await expect(page.getByLabel('Auto-reply subject')).toBeVisible()
    await expect(page.getByLabel('Auto-reply message')).toBeVisible()
    await page.getByLabel('Auto-reply subject').fill('On leave until Friday')
    await expect(page.getByLabel('Auto-reply subject')).toHaveValue('On leave until Friday')

    await page.getByRole('checkbox', { name: /Forward incoming messages/ }).check()
    await page.getByLabel('Forwarding address').fill('forward@example.com')
    await page.getByRole('checkbox', { name: /Keep a copy in my mailbox/ }).check()

    await page.keyboard.press('Escape')
    await expect(settings).toBeHidden()
    await expect(page).toHaveURL(/\/mail\/inbox$/)
  })

  test('creates a filter rule', async ({ page }) => {
    const settings = await openSettings(page)
    await settings.getByRole('button', { name: 'Filters', exact: true }).click()
    await expect(page.getByText('Newsletters')).toBeVisible()

    await page.getByRole('button', { name: 'New rule' }).click()
    await page.getByLabel('Rule name').fill('Big client')
    await page.getByLabel('From value').fill('boss@harbor.co')
    await page.getByRole('button', { name: 'Save rule' }).click()
    await expect(page.getByText('Big client', { exact: true })).toBeVisible()
  })

  test('blocks and allows senders', async ({ page }) => {
    const settings = await openSettings(page)
    await settings.getByRole('button', { name: 'Spam', exact: true }).click()

    await page.getByLabel('Block an address').fill('nagging@example.com')
    await page.getByRole('button', { name: 'Block', exact: true }).click()
    await expect(page.getByText('nagging@example.com')).toBeVisible()

    await page.getByLabel('Allow an address').fill('trusted@example.com')
    await page.getByRole('button', { name: 'Allow', exact: true }).click()
    await expect(page.getByText('trusted@example.com')).toBeVisible()
  })

  test('adds an identity and offers it in the composer', async ({ page }) => {
    const settings = await openSettings(page)
    await settings.getByRole('button', { name: 'Identities', exact: true }).click()

    await page.getByLabel('Identity name').fill('Marketing')
    await page.getByLabel('Identity email').fill('marketing@harbor.co')
    await page.getByRole('button', { name: 'Add', exact: true }).click()
    await expect(page.getByText('marketing@harbor.co', { exact: true })).toBeVisible()

    await page.keyboard.press('Escape')
    await expect(settings).toBeHidden()
    await page.keyboard.press('c')
    const from = page.getByRole('combobox', { name: 'From address' })
    await expect(from).toBeVisible()
    await expect(from.locator('option', { hasText: 'marketing@harbor.co' })).toBeEnabled()
  })
})
