import { expect, test } from '@playwright/test'
import { openInbox } from './helpers'

test.describe('admin center', () => {
  test('manages mailboxes, aliases and forwarders', async ({ page }) => {
    await openInbox(page)
    await page.goto('/mail/admin')
    const admin = page.getByRole('region', { name: 'Admin center' })
    await expect(admin).toBeVisible()
    await expect(admin.getByRole('heading', { name: 'Admin center' })).toBeVisible()
    await expect(admin.getByRole('button', { name: 'Overview', exact: true })).toHaveAttribute('aria-current', 'page')

    await admin.getByRole('button', { name: 'Mailboxes', exact: true }).click()
    await expect(admin.getByText('alex@harbor.co').first()).toBeVisible()

    await admin.getByLabel('Mailbox address').fill('ops')
    await admin.getByLabel('Mailbox display name').fill('Ops Team')
    await admin.getByRole('button', { name: 'Add mailbox' }).click()
    await expect(admin.getByText('ops@harbor.co', { exact: true })).toBeVisible()
    await expect(admin.getByText('Mailbox ops@harbor.co created')).toBeVisible()

    await admin.getByRole('button', { name: 'Aliases', exact: true }).click()
    await expect(admin.getByText('support@harbor.co')).toBeVisible()
    await admin.getByLabel('Alias address').fill('team')
    await admin.getByLabel('Alias target').selectOption('alex@harbor.co')
    await admin.getByRole('button', { name: 'Add alias' }).click()
    await expect(admin.getByText('team@harbor.co', { exact: true })).toBeVisible()

    await admin.getByRole('button', { name: 'Forwarders', exact: true }).click()
    await expect(admin.getByText('alexmorgan@example.com')).toBeVisible()
    await admin.getByLabel('Forwarder target').fill('external@example.net')
    await admin.getByRole('button', { name: 'Add forwarder' }).click()
    await expect(admin.getByText('external@example.net')).toBeVisible()
    const newForwarder = admin.locator('.billing-row', { hasText: 'external@example.net' })
    await newForwarder.getByRole('button', { name: 'Pause' }).click()
    await expect(newForwarder).toContainText('Paused')
    await newForwarder.getByRole('button', { name: 'Enable' }).click()
    await expect(newForwarder).toContainText('Enabled')

    await admin.getByRole('button', { name: 'Mailboxes', exact: true }).click()
    await admin.getByRole('button', { name: 'Remove ops@harbor.co' }).click()
    await expect(admin.getByText('ops@harbor.co', { exact: true })).toBeHidden()
    await expect(admin.getByText('Mailbox removed')).toBeVisible()
  })

  test('searches and filters mailboxes by status', async ({ page }) => {
    await openInbox(page)
    await page.goto('/mail/admin')
    const admin = page.getByRole('region', { name: 'Admin center' })
    await admin.getByRole('button', { name: 'Mailboxes', exact: true }).click()

    await admin.getByRole('textbox', { name: 'Search mailboxes' }).fill('nora')
    await expect(admin.getByText('nora@harbor.co', { exact: true })).toBeVisible()
    await expect(admin.getByText('alex@harbor.co', { exact: true })).toBeHidden()

    await admin.getByRole('combobox', { name: 'Filter mailboxes by status' }).selectOption('disabled')
    await expect(admin.getByText('No mailboxes match this search.')).toBeVisible()
  })

  test('verifies DNS records and sets a catch-all', async ({ page }) => {
    await openInbox(page)
    await page.goto('/mail/admin')
    const admin = page.getByRole('region', { name: 'Admin center' })
    await admin.getByRole('button', { name: 'Domain', exact: true }).click()
    await expect(admin.getByRole('heading', { name: 'harbor.co' })).toBeVisible()
    await expect(admin.getByText('0 of 4 records verified')).toBeVisible()
    await expect(admin.getByText('Required').first()).toBeVisible()

    await admin.getByRole('button', { name: 'Check DNS records' }).click()
    await expect(admin.getByText('All records verified — mail is flowing.')).toBeVisible()
    await expect(admin.getByText('Verified').first()).toBeVisible()

    await admin.getByLabel('Deliver mail for unknown addresses at harbor.co').check()
    await admin.getByLabel('Catch-all recipient').selectOption('dev@harbor.co')
    await expect(admin.getByLabel('Catch-all recipient')).toHaveValue('dev@harbor.co')
  })

  test('tunes security policy, blocks a sender, and records it in the audit log', async ({ page }) => {
    await openInbox(page)
    await page.goto('/mail/admin')
    const admin = page.getByRole('region', { name: 'Admin center' })

    await admin.getByRole('button', { name: 'Security', exact: true }).click()
    const slider = admin.getByRole('slider', { name: 'Spam threshold' })
    await slider.fill('8')
    await expect(slider).toHaveValue('8')
    await admin.getByLabel('DMARC policy').selectOption('reject')

    await admin.getByRole('textbox', { name: 'Blocked sender' }).fill('scammer@bad.example')
    await admin.getByRole('button', { name: 'Block sender' }).click()
    await expect(admin.getByText('scammer@bad.example')).toBeVisible()

    await admin.getByRole('button', { name: 'Audit log', exact: true }).click()
    await expect(admin.getByText('Spam threshold changed')).toBeVisible()
    await expect(admin.getByText('DMARC policy changed')).toBeVisible()
    await expect(admin.getByText('Blocked sender added')).toBeVisible()
  })

  test('reviews the quarantine and releases one message to the inbox', async ({ page }) => {
    await openInbox(page)
    await page.goto('/mail/admin')
    const admin = page.getByRole('region', { name: 'Admin center' })

    await admin.getByRole('button', { name: 'Quarantine', exact: true }).click()
    await expect(admin.getByText('You have won a prize!')).toBeVisible()

    const prize = admin.locator('.billing-row', { hasText: 'You have won a prize!' })
    await prize.getByRole('button', { name: 'Release' }).click()
    await expect(admin.getByText('Message released — delivered to Inbox')).toBeVisible()
    await expect(admin.getByText('You have won a prize!')).toBeHidden()

    await page.goto('/mail/inbox')
    await expect(page.locator('.mail-row', { hasText: 'You have won a prize!' })).toBeVisible()

    await page.goto('/mail/admin')
    const adminAgain = page.getByRole('region', { name: 'Admin center' })
    await adminAgain.getByRole('button', { name: 'Quarantine', exact: true }).click()
    await adminAgain.getByRole('button', { name: 'Delete quarantined message Invoice overdue — pay immediately' }).click()
    await expect(adminAgain.getByText('Nothing quarantined right now.')).toBeVisible()
  })

  test('opens the admin center from the command palette and Esc closes it', async ({ page }) => {
    await openInbox(page)
    await page.keyboard.press('Control+k')
    await page.getByRole('combobox', { name: 'Search commands' }).fill('admin')
    await page.getByRole('option', { name: 'Open admin panel' }).click()
    await expect(page.getByRole('region', { name: 'Admin center' })).toBeVisible()
    await page.keyboard.press('Escape')
    await expect(page.getByRole('region', { name: 'Admin center' })).toBeHidden()
  })
})
