import { expect, test } from '@playwright/test'
import { openInbox } from './helpers'

test.describe('profile menu', () => {
  test('opens settings and billing from the account menu', async ({ page }) => {
    await openInbox(page)

    await page.getByRole('button', { name: 'Open account menu' }).click()
    const menu = page.getByRole('menu')
    await expect(menu).toBeVisible()
    await menu.getByRole('menuitem', { name: 'Billing', exact: true }).click()
    const billing = page.getByRole('region', { name: 'Billing' })
    await expect(billing).toBeVisible()
    await expect(page).toHaveURL(/\/mail\/billing/)
    await expect(billing).toContainText('Harbor Team')
    await page.keyboard.press('Escape')
    await expect(billing).toBeHidden()

    await page.getByRole('button', { name: 'Open account menu' }).click()
    await menu.getByRole('menuitem', { name: 'Settings', exact: true }).click()
    const settings = page.getByRole('region', { name: 'Settings' })
    await expect(settings).toBeVisible()
    await expect(page).toHaveURL(/\/mail\/settings/)
    await page.keyboard.press('Escape')
    await expect(settings).toBeHidden()
    await expect(page).toHaveURL(/\/mail\/inbox$/)
  })
})
