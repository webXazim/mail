import { expect, test } from '@playwright/test'
import { openInbox, openThread } from './helpers'

test.use({ viewport: { width: 390, height: 844 } })

const noPageOverflow = (page: import('@playwright/test').Page) =>
  page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth)

test.describe('mobile polish', () => {
  test('opens and closes the navigation drawer from the hamburger', async ({ page }) => {
    await openInbox(page)
    const sidebar = page.locator('.sidebar')
    await expect(sidebar).not.toHaveClass(/sidebar--open/)
    await page.getByRole('button', { name: 'Open navigation' }).click()
    await expect(sidebar).toHaveClass(/sidebar--open/)
    await sidebar.getByRole('button', { name: 'Close navigation' }).click()
    await expect(sidebar).not.toHaveClass(/sidebar--open/)
  })

  test('keeps every list-toolbar action reachable with no horizontal overflow', async ({ page }) => {
    await openInbox(page)
    const overflow = await page.locator('.mail-toolbar').evaluate(el => el.scrollWidth - el.clientWidth)
    expect(overflow).toBeLessThanOrEqual(2)
    await expect(page.getByRole('button', { name: 'Sort messages' })).toBeVisible()
    await expect(page.getByRole('button', { name: 'Archive', exact: true })).toBeVisible()
    expect(await noPageOverflow(page)).toBeLessThanOrEqual(2)
  })

  test('shows a labeled back button and reachable actions in the reader', async ({ page }) => {
    await openThread(page, 'Nora Li')
    const back = page.getByRole('button', { name: 'Back' })
    await expect(back).toBeVisible()
    await expect(back).toContainText('Back')
    const overflow = await page.locator('.reader-toolbar').evaluate(el => el.scrollWidth - el.clientWidth)
    expect(overflow).toBeLessThanOrEqual(2)
    await expect(page.getByRole('button', { name: 'Report spam' })).toBeVisible()
  })

  test('composes from a floating button with the footer fully visible', async ({ page }) => {
    await openInbox(page)
    const compose = page.locator('.page-head').getByRole('button', { name: 'Compose' })
    await expect(compose).toBeVisible()
    await compose.click()
    const dialog = page.getByRole('dialog', { name: 'New message' })
    await expect(dialog).toBeVisible()
    const box = await dialog.boundingBox()
    expect(box!.width).toBeLessThanOrEqual(390)
    await expect(page.getByRole('button', { name: /Send/ })).toBeVisible()
  })
})