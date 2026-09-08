import { expect, test } from '@playwright/test'
import { boot } from './helpers'

test.describe('sign in', () => {
  test('renders the branded login page', async ({ page }) => {
    await boot(page)
    await page.goto('/login')
    await expect(page).toHaveTitle(/Harbor Mail/)
    await expect(page.getByText('Sign in to your mailbox')).toBeVisible()
    await expect(page.locator('.auth-brand')).toContainText('harbor')
    await expect(page.getByText('New to Harbor Mail?')).toBeVisible()
  })

  test('signs in and lands on the inbox', async ({ page }) => {
    await boot(page)
    await page.goto('/login')
    await page.locator('input[name="email"]').fill('you@harbor.co')
    await page.locator('input[name="password"]').fill('password123')
    await page.getByRole('button', { name: /Sign in/ }).click()
    await expect(page).toHaveURL(/\/mail\/inbox/)
    await expect(page.getByRole('heading', { name: 'Inbox', exact: true })).toBeVisible()
    await expect(page.getByText('Nora Li')).toBeVisible()
  })

  test('rejects an invalid password', async ({ page }) => {
    await boot(page)
    await page.goto('/login')
    await page.locator('input[name="email"]').fill('you@harbor.co')
    await page.locator('input[name="password"]').fill('123')
    await page.getByRole('button', { name: /Sign in/ }).click()
    await expect(page.getByRole('alert')).toContainText('Invalid email or password')
  })
})