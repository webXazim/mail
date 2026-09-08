import { expect, test } from '@playwright/test'
import { openInbox } from './helpers'

const linkAccount = async (page: import('@playwright/test').Page) => {
  await page.getByRole('button', { name: 'Manage accounts' }).click()
  const settings = page.getByRole('dialog', { name: 'Settings' })
  await expect(settings).toBeVisible()
  await settings.getByLabel('Display name').fill('Amira Khalil')
  await settings.getByLabel('Email address').fill('amira@northbeam.dev')
  await settings.getByLabel('Password').fill('secret123')
  await settings.getByRole('button', { name: 'Link account' }).click()
  await expect(settings.getByText('amira@northbeam.dev')).toBeVisible()
  return settings
}

test.describe('multi-account / unified inbox', () => {
  test('links an account, merges its mail, and focuses each account alone', async ({ page }) => {
    await openInbox(page)
    const settings = await linkAccount(page)
    await settings.getByRole('button', { name: 'Close settings' }).click()

    const accountButton = page.locator('.sidebar').getByRole('button', { name: /amira@northbeam\.dev/ })
    await expect(accountButton).toBeVisible()
    await expect(page.locator('.mail-row', { hasText: 'Maya Chen' })).toBeVisible()
    await expect(page.locator('.mail-row', { hasText: 'Nora Li' })).toBeVisible()

    await accountButton.click()
    await expect(page.locator('.mail-row', { hasText: 'Maya Chen' })).toBeVisible()
    await expect(page.locator('.mail-row', { hasText: 'Nora Li' })).toHaveCount(0)

    await page.locator('.sidebar').getByRole('button', { name: 'Unified inbox' }).click()
    await expect(page.locator('.mail-row', { hasText: 'Nora Li' })).toBeVisible()
    await expect(page.locator('.mail-row', { hasText: 'Maya Chen' })).toBeVisible()
  })

  test('removes an account and its messages leave the unified inbox', async ({ page }) => {
    await openInbox(page)
    const settings = await linkAccount(page)
    await settings.getByRole('button', { name: 'Remove' }).click()
    await expect(settings.getByText('amira@northbeam.dev')).toHaveCount(0)
    await settings.getByRole('button', { name: 'Close settings' }).click()

    await expect(page.locator('.sidebar').getByRole('button', { name: /amira@northbeam\.dev/ })).toHaveCount(0)
    await expect(page.locator('.mail-row', { hasText: 'Maya Chen' })).toHaveCount(0)
    await expect(page.locator('.mail-row', { hasText: 'Nora Li' })).toBeVisible()
  })
})