import { expect, test, type Page } from '@playwright/test'
import { openInbox } from './helpers'

const composeButton = (page: Page) => page.locator('.page-head').getByRole('button', { name: 'Compose' })

async function openFilters(page: Page) {
  await openInbox(page)
  await page.getByRole('button', { name: 'Open account menu' }).click()
  await page.getByRole('menuitem', { name: 'Settings', exact: true }).click()
  const settings = page.getByRole('region', { name: 'Settings' })
  await expect(settings).toBeVisible()
  await settings.getByRole('button', { name: 'Filters', exact: true }).click()
  await expect(page.getByText('Newsletters')).toBeVisible()
}

const sidebarButton = async (page: Page, folder: string) => {
  const sidebar = page.locator('.sidebar')
  const moreFolders = sidebar.getByRole('button', { name: 'More', exact: true })
  if (!(await sidebar.getByRole('button', { name: new RegExp(`^${folder}\\s`) }).count())) await moreFolders.click()
  return sidebar.getByRole('button', { name: new RegExp(`^${folder}\\s`) })
}

async function addRule(page: Page, name: string, from: string, actionKind: string) {
  await page.getByRole('button', { name: 'New rule' }).click()
  await page.getByLabel('Rule name').fill(name)
  await page.getByLabel('From value').fill(from)
  await page.getByLabel('Action kind').selectOption(actionKind)
  await page.getByRole('button', { name: 'Save rule' }).click()
  await expect(page.getByText(name, { exact: true })).toBeVisible()
}

async function sendComposed(page: Page, recipients: string, subject: string) {
  await composeButton(page).click()
  const composer = page.getByRole('dialog', { name: 'New message' })
  await expect(composer).toBeVisible()
  await page.getByPlaceholder('Recipients').fill(recipients)
  await page.getByPlaceholder('Subject').fill(subject)
  await page.getByRole('textbox', { name: 'Message body' }).fill('Hello there.')
  await page.getByRole('button', { name: 'Send' }).click()
  await expect(page.getByRole('status').filter({ hasText: 'Message sent' })).toBeVisible()
}

test.describe('rules parity', () => {
  test('routes a delivered reply through an incoming-archive rule', async ({ page }) => {
    await openFilters(page)
    await addRule(page, 'Archive Nora', 'northstar.studio', 'archive')
    await page.goto('/mail/inbox')
    await expect(page.getByRole('heading', { name: 'Inbox', exact: true })).toBeVisible()

    await sendComposed(page, 'nora@northstar.studio', 'Rules parity')
    await expect(page.getByRole('status').filter({ hasText: 'New mail from Nora Li' })).toBeVisible({ timeout: 15000 })

    await (await sidebarButton(page, 'Archive')).click()
    await expect(page.getByRole('heading', { name: 'Archive', exact: true })).toBeVisible()
    await expect(page.locator('.mail-row', { hasText: 'Re: Rules parity' })).toBeVisible()

    await (await sidebarButton(page, 'Inbox')).click()
    await expect(page.getByRole('heading', { name: 'Inbox', exact: true })).toBeVisible()
    await expect(page.locator('.mail-row', { hasText: 'Re: Rules parity' })).toHaveCount(0)
  })

  test('discards a delivered reply that matches a discard rule and stays silent', async ({ page }) => {
    await openFilters(page)
    await addRule(page, 'Junk mail', 'elsewhere.dev', 'discard')
    await page.goto('/mail/inbox')
    await expect(page.getByRole('heading', { name: 'Inbox', exact: true })).toBeVisible()

    await sendComposed(page, 'jo@elsewhere.dev', 'Junk check')
    await expect(page.getByRole('status').filter({ hasText: 'Discarded an unwanted message from Jo' })).toBeVisible({ timeout: 15000 })
    await expect(page.getByRole('status').filter({ hasText: 'New mail from Jo' })).toHaveCount(0)

    await (await sidebarButton(page, 'Trash')).click()
    await expect(page.getByRole('heading', { name: 'Trash', exact: true })).toBeVisible()
    await expect(page.locator('.mail-row', { hasText: 'Re: Junk check' })).toBeVisible()

    await (await sidebarButton(page, 'Inbox')).click()
    await expect(page.getByRole('heading', { name: 'Inbox', exact: true })).toBeVisible()
    await expect(page.getByText('Re: Junk check')).toHaveCount(0)
  })
})
