import { expect, test } from '@playwright/test'
import { openInbox } from './helpers'

test.describe('admin', () => {
  test('manages mailboxes, aliases and forwarders', async ({ page }) => {
    await openInbox(page)
    await page.goto('/mail/admin')
    const admin = page.getByRole('dialog', { name: 'Admin panel' })
    await expect(admin).toBeVisible()
    await expect(admin.getByRole('button', { name: 'Mailboxes', exact: true })).toHaveAttribute('aria-current', 'page')
    await expect(admin.getByText('alex@harbor.co').first()).toBeVisible()
    await expect(admin.getByText('Active').first()).toBeVisible()

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

  test('verifies DNS records and sets a catch-all', async ({ page }) => {
    await openInbox(page)
    await page.goto('/mail/admin')
    const admin = page.getByRole('dialog', { name: 'Admin panel' })
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

  test('opens the admin panel from the command palette and Esc closes it', async ({ page }) => {
    await openInbox(page)
    await page.keyboard.press('Control+k')
    await page.getByRole('combobox', { name: 'Search commands' }).fill('admin')
    await page.getByRole('option', { name: 'Open admin panel' }).click()
    await expect(page.getByRole('dialog', { name: 'Admin panel' })).toBeVisible()
    await page.keyboard.press('Escape')
    await expect(page.getByRole('dialog', { name: 'Admin panel' })).toBeHidden()
  })
})