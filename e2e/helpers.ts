import { expect, type Page } from '@playwright/test'

export async function boot(page: Page) {
  await page.addInitScript(() => {
    localStorage.setItem('harbor-mail:onboarded', '1')
    localStorage.setItem('harbor-mail:session', 'demo')
  })
}

export async function openInbox(page: Page) {
  await boot(page)
  await page.goto('/mail/inbox')
  await expect(page.getByRole('heading', { name: 'Inbox', exact: true })).toBeVisible()
  await expect(page.getByText('Nora Li')).toBeVisible()
}

export const inboxRow = (page: Page, subject: string) => page.locator('.mail-row', { hasText: subject })

export async function openThread(page: Page, sender: string) {
  await openInbox(page)
  await page.locator('.mail-row', { hasText: sender }).getByRole('button', { name: /^Open / }).click()
  await expect(page).toHaveURL(/\/thread\//)
}