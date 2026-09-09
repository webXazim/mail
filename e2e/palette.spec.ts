import { expect, test } from '@playwright/test'
import { openInbox } from './helpers'

test.describe('command palette', () => {
  test('navigates to a folder from the palette', async ({ page }) => {
    await openInbox(page)
    await page.keyboard.press('Control+k')
    const palette = page.getByRole('dialog', { name: 'Command palette' })
    await expect(palette).toBeVisible()
    await page.getByPlaceholder('Type a command or folder name...').fill('sent')
    await page.keyboard.press('Enter')
    await expect(page).toHaveURL(/\/mail\/sent/)
    await expect(page.getByRole('heading', { name: 'Sent', exact: true })).toBeVisible()
  })

  test('opens settings from the palette', async ({ page }) => {
    await openInbox(page)
    await page.keyboard.press('Control+k')
    await page.getByPlaceholder('Type a command or folder name...').fill('options')
    await page.keyboard.press('Enter')
    const settings = page.getByRole('region', { name: 'Settings' })
    await expect(settings).toBeVisible()
    await page.keyboard.press('Escape')
    await expect(settings).toBeHidden()
  })
})