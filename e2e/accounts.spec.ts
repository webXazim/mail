import { expect, test } from '@playwright/test'
import { openInbox } from './helpers'

const linkAccount = async (page: import('@playwright/test').Page) => {
  await page.getByRole('button', { name: 'Open account menu' }).click()
  await page.getByRole('menu').getByRole('button', { name: 'Manage accounts' }).click()
  const settings = page.getByRole('region', { name: 'Settings' })
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
    await page.keyboard.press('Escape')

    await page.getByRole('button', { name: 'Open account menu' }).click()
    const accountMenu = page.getByRole('menu')
    const accountButton = accountMenu.getByRole('button', { name: /amira@northbeam\.dev/ })
    await expect(accountButton).toBeVisible()
    await expect(page.locator('.mail-row', { hasText: 'Maya Chen' })).toBeVisible()
    await expect(page.locator('.mail-row', { hasText: 'Nora Li' })).toBeVisible()

    await accountButton.click()
    await expect(page.locator('.mail-row', { hasText: 'Maya Chen' })).toBeVisible()
    await expect(page.locator('.mail-row', { hasText: 'Nora Li' })).toHaveCount(0)

    await page.getByRole('button', { name: 'Open account menu' }).click()
    await page.getByRole('menu').getByRole('button', { name: 'Switch to unified inbox' }).click()
    await expect(page.locator('.mail-row', { hasText: 'Nora Li' })).toBeVisible()
    await expect(page.locator('.mail-row', { hasText: 'Maya Chen' })).toBeVisible()
  })

  test('removes an account and its messages leave the unified inbox', async ({ page }) => {
    await openInbox(page)
    const settings = await linkAccount(page)
    await settings.getByRole('button', { name: 'Remove' }).click()
    await expect(settings.getByText('amira@northbeam.dev')).toHaveCount(0)
    await page.keyboard.press('Escape')

    await page.getByRole('button', { name: 'Open account menu' }).click()
    await expect(page.getByRole('menu').getByRole('button', { name: /amira@northbeam\.dev/ })).toHaveCount(0)
    await expect(page.locator('.mail-row', { hasText: 'Maya Chen' })).toHaveCount(0)
    await expect(page.locator('.mail-row', { hasText: 'Nora Li' })).toBeVisible()
  })
})
