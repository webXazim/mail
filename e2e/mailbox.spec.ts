import { expect, test } from '@playwright/test'
import { openInbox } from './helpers'

test.describe('mailbox tools', () => {
  test('sorts and filters the list from the toolbar', async ({ page }) => {
    await openInbox(page)

    await page.getByRole('button', { name: 'Filter messages' }).click()
    await page.getByRole('menuitemcheckbox', { name: 'Starred' }).click()
    await expect(page.getByText('Nora Li')).toBeVisible()
    await expect(page.getByText('Jonas Meier')).toBeHidden()
    await expect(page.locator('.toolbar-count')).toHaveText('1')

    await page.getByRole('menuitem', { name: 'Clear filters' }).click()
    await expect(page.getByText('Jonas Meier')).toBeVisible()

    await page.getByRole('button', { name: 'Sort messages' }).click()
    await page.getByRole('menuitemradio', { name: 'Sender (A to Z)' }).click()
    await expect(page.locator('.mail-list .mail-row').first()).toContainText('Jonas Meier')
  })

  test('moves a conversation to trash, then empties trash', async ({ page }) => {
    await openInbox(page)
    await page.getByLabel('Select Design review notes').check()
    await page.getByLabel('Move to folder').selectOption('Trash')
    await expect(page.getByText('Priya Shah')).toBeHidden()

    await page.locator('.sidebar').getByRole('button', { name: /Trash/ }).click()
    await expect(page.getByRole('heading', { name: 'Trash', exact: true })).toBeVisible()
    await expect(page.getByText('Design review notes')).toBeVisible()
    await page.getByRole('button', { name: 'Empty trash' }).click()
    await page.getByRole('menuitem', { name: 'Permanently delete all trash' }).click()
    await expect(page.getByText('Nothing here yet')).toBeVisible()
  })

  test('recovers a message from spam', async ({ page }) => {
    await openInbox(page)
    await page.goto('/mail/spam')
    await expect(page.getByText('New sign-in to your Harbor account')).toBeVisible()
    await page.getByRole('button', { name: 'Not spam' }).click()
    await expect(page.getByText('Nothing here yet')).toBeVisible()
    await expect(page.getByRole('status').filter({ hasText: 'Moved to Inbox' })).toBeVisible()
  })
})

test.describe('labels', () => {
  test('adds and removes a label from the sidebar', async ({ page }) => {
    await openInbox(page)

    await page.getByRole('button', { name: 'Manage labels' }).click()
    const dialog = page.getByRole('dialog', { name: 'Manage labels' })
    await expect(dialog).toBeVisible()

    await page.getByLabel('New label name').fill('Urgent')
    await page.getByRole('button', { name: 'Add', exact: true }).click()
    await expect(page.getByLabel('Rename Urgent')).toBeVisible()

    await page.getByRole('button', { name: 'Done' }).click()
    await page.getByRole('button', { name: /^Urgent$/ }).first().click()
    await expect(page.getByRole('heading', { name: 'Inbox', exact: true })).toBeVisible()

    await page.getByRole('button', { name: 'Manage labels' }).click()
    await page.getByRole('button', { name: 'Delete Urgent' }).click()
    await expect(page.getByLabel('Rename Urgent')).toBeHidden()
  })
})

test.describe('billing', () => {
  test('switches plan and downloads an invoice', async ({ page }) => {
    await openInbox(page)
    await page.goto('/mail/billing')
    const billing = page.getByRole('dialog', { name: 'Billing' })
    await expect(billing).toBeVisible()

    await billing.getByRole('button', { name: 'Invoices', exact: true }).click()
    await expect(billing.getByText('INV-2026-036')).toBeVisible()
    await billing.getByRole('button', { name: 'Download' }).first().click()
    await expect(billing.getByText('Sent to your inbox').first()).toBeVisible()

    await billing.getByRole('button', { name: 'Plan', exact: true }).click()
    await billing.getByRole('button', { name: 'Change plan' }).click()
    await billing.getByRole('button', { name: /Harbor Business/ }).click()
    await billing.getByRole('button', { name: 'Apply plan' }).click()
    await expect(billing.getByText('Your plan is now Harbor Business.')).toBeVisible()
  })
})

test.describe('settings tabs', () => {
  test('notifications and security tabs work', async ({ page }) => {
    await openInbox(page)
    await page.goto('/mail/settings')
    const settings = page.getByRole('dialog', { name: 'Settings' })
    await expect(settings).toBeVisible()

    await settings.getByRole('button', { name: 'Notifications', exact: true }).click()
    await expect(settings.getByText('Browser permission')).toBeVisible()
    await expect(settings.getByText('Play an alert sound')).toBeVisible()

    await settings.getByRole('button', { name: 'Security', exact: true }).click()
    await expect(settings.getByText('Change password')).toBeVisible()

    await settings.getByLabel('Current password', { exact: true }).fill('wrong')
    await settings.getByLabel('New password', { exact: true }).fill('hunter2hunter')
    await settings.getByLabel('Confirm new password', { exact: true }).fill('hunter2hunter')
    await settings.getByRole('button', { name: 'Update password' }).click()
    await expect(settings.getByText('Current password is incorrect')).toBeVisible()

    await settings.getByLabel('Current password', { exact: true }).fill('current-password')
    await settings.getByRole('button', { name: 'Update password' }).click()
    await expect(settings.getByText('Password updated')).toBeVisible()
  })
})

test.describe('composer validation', () => {
  test('flags invalid recipients and sends once fixed', async ({ page }) => {
    await openInbox(page)
    await page.locator('.page-head').getByRole('button', { name: 'Compose' }).click()
    const composer = page.getByRole('dialog', { name: 'New message' })
    await expect(composer).toBeVisible()

    const recipients = page.getByPlaceholder('Recipients')
    await recipients.fill('not-an-email')
    await page.getByPlaceholder('Subject').fill('Quick note')
    await page.getByRole('button', { name: 'Send' }).click()
    await expect(page.getByText('Check the recipients: enter valid email addresses')).toBeVisible()
    await expect(page.locator('.composer-error')).toContainText('Enter a valid email address')

    await recipients.fill('nora@northstar.studio')
    await page.getByRole('button', { name: 'Send' }).click()
    await expect(composer).toBeHidden()
  })
})

test.describe('reader actions', () => {
  test('moves an open conversation with the reader toolbar', async ({ page }) => {
    await openInbox(page)
    await page.getByRole('button', { name: 'Open Q3 launch plan - review before Thursday' }).click()
    await expect(page.getByRole('heading', { name: 'Q3 launch plan - review before Thursday' })).toBeVisible()

    await page.getByRole('button', { name: 'Move to folder' }).click()
    await page.getByRole('menuitem', { name: 'Archive', exact: true }).click()
    await expect(page.getByRole('heading', { name: 'Archive', exact: true })).toBeVisible()
    await expect(page.getByText('Q3 launch plan - review before Thursday')).toBeVisible()
    await expect(page.getByRole('status').filter({ hasText: 'Moved to Archive' })).toBeVisible()
  })

  test('marks the whole inbox read from the toolbar', async ({ page }) => {
    await openInbox(page)
    const unreadRow = page.locator('.sidebar').getByRole('button', { name: /^Unread/ })
    await expect(unreadRow.locator('b')).toHaveText('3')

    await page.getByRole('button', { name: 'Mark all read' }).click()
    await expect(page.locator('.mail-row--unread')).toHaveCount(0)
    await expect(unreadRow.locator('b')).toBeHidden()
  })
})

test.describe('scheduled messages', () => {
  test('cancels a scheduled message from the folder', async ({ page }) => {
    await openInbox(page)
    await page.locator('.page-head').getByRole('button', { name: 'Compose' }).click()
    const composer = page.getByRole('dialog', { name: 'New message' })
    await page.getByPlaceholder('Recipients').fill('jonas@meridian.co')
    await page.getByPlaceholder('Subject').fill('Deferred note')
    await page.getByLabel('Schedule message').fill('2026-09-08T12:00')
    await page.getByRole('button', { name: 'Schedule', exact: true }).click()
    await expect(composer).toBeHidden()

    await page.locator('.sidebar').getByRole('button', { name: /Scheduled/ }).click()
    await expect(page.getByRole('heading', { name: 'Scheduled', exact: true })).toBeVisible()
    await expect(page.getByText('Deferred note')).toBeVisible()

    const row = page.locator('.mail-row').filter({ hasText: 'Deferred note' })
    await row.hover()
    await expect(row.locator('button[aria-label="Cancel Deferred note"]')).toBeVisible()
    await row.locator('button[aria-label="Cancel Deferred note"]').click({ force: true })
    await expect(page.getByText('Deferred note')).toBeHidden()
    await expect(page.locator('.list-state')).toContainText('Nothing scheduled')
  })
})

test.describe('login flows', () => {
  test('recovers a forgotten password', async ({ page }) => {
    await page.goto('/login')
    await page.getByRole('button', { name: 'Forgot password?' }).click()
    await expect(page.getByRole('heading', { name: 'Reset your password' })).toBeVisible()

    await page.getByLabel('Email address').fill('alex@harbor.co')
    await page.getByRole('button', { name: 'Send reset link' }).click()
    await expect(page.getByRole('status')).toContainText('a reset link is on its way')

    await page.getByRole('button', { name: 'Back to sign in' }).click()
    await expect(page.getByRole('heading', { name: 'Sign in to your mailbox' })).toBeVisible()
  })

  test('creates an account and signs in', async ({ page }) => {
    await page.goto('/login')
    await page.getByRole('button', { name: 'Create an account' }).click()
    await expect(page.getByRole('heading', { name: 'Create your account' })).toBeVisible()

    await page.getByLabel('Full name').fill('Test User')
    await page.getByLabel('Email address').fill('test@harbor.co')
    await page.getByLabel('Password', { exact: true }).fill('test-password')
    await page.getByLabel('Confirm password').fill('test-password')
    await page.getByRole('button', { name: 'Create account', exact: true }).click()
    await expect(page.getByRole('heading', { name: 'Inbox', exact: true })).toBeVisible()
  })
})