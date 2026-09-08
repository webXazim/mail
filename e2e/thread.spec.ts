import { expect, test } from '@playwright/test'
import { openThread } from './helpers'

test.describe('thread', () => {
  test('opens a conversation in the split reader', async ({ page }) => {
    await openThread(page, 'Nora Li')
    await expect(page.locator('.split-list')).toBeVisible()
    await expect(page.locator('.reader-content')).toContainText('Q3 launch plan - review before Thursday')
    await expect(page.locator('.reader')).toContainText('folded your feedback into the latest version')
  })

  test('backs out to the folder list', async ({ page }) => {
    await openThread(page, 'Nora Li')
    await page.getByRole('button', { name: 'Back' }).click()
    await expect(page).toHaveURL(/\/mail\/inbox$/)
    await expect(page.getByRole('heading', { name: 'Inbox', exact: true })).toBeVisible()
  })

  test('starts a reply with the keyboard', async ({ page }) => {
    await openThread(page, 'Nora Li')
    await expect(page.locator('.reader-content')).toContainText('Q3 launch plan - review before Thursday')
    const composer = page.getByRole('dialog', { name: 'New message' })
    await page.locator('.reply-box').click()
    await expect(composer).toBeVisible()
    await page.keyboard.press('Escape')
    await expect(composer).toBeHidden()
    await page.keyboard.press('r')
    await expect(composer).toBeVisible()
    await expect(page.getByPlaceholder('Recipients')).toHaveValue(/nora@northstar.studio/)
  })
})