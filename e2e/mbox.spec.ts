import { expect, test } from '@playwright/test'
import { readFileSync } from 'node:fs'
import { existsSync, mkdirSync } from 'node:fs'
import { openInbox } from './helpers'

const sampleMbox = [
  'From alex@harbor.co Just now',
  'From: Sam Rivera <sam@elsewhere.dev>',
  'Subject: Imported Standup Notes',
  'Date: Just now',
  'X-Harbor-Id: import-a',
  'X-Harbor-Initials: SR',
  'X-Harbor-Sender: Sam Rivera',
  'X-Harbor-Sender-Email: sam@elsewhere.dev',
  'X-Harbor-Color: teal',
  'X-Harbor-Folder: Inbox',
  'X-Harbor-Label: Updates',
  'X-Harbor-Unread: yes',
  'X-Harbor-Starred: no',
  'X-Harbor-Attachment: no',
  '',
  'Standup moved to tomorrow.',
  '',
  'From alex@harbor.co 9:30 AM',
  'From: Jo Park <jo@elsewhere.dev>',
  'Subject: Imported Design Sync',
  'Date: 9:30 AM',
  'X-Harbor-Id: import-b',
  'X-Harbor-Initials: JP',
  'X-Harbor-Sender: Jo Park',
  'X-Harbor-Sender-Email: jo@elsewhere.dev',
  'X-Harbor-Color: purple',
  'X-Harbor-Folder: Archive',
  'X-Harbor-Label: Social',
  'X-Harbor-Unread: no',
  'X-Harbor-Starred: yes',
  'X-Harbor-Attachment: no',
  '',
  'Round up on the new design.',
].join('\n')

const resultDir = 'test-results'
const exportFile = `${resultDir}/harbor-mail-export.mbox`

test.describe('mbox export and import', () => {
  test('exports the mailbox as an .mbox file', async ({ page }) => {
    await openInbox(page)
    await page.goto('/mail/all')
    await expect(page.getByRole('heading', { name: 'All Mail', exact: true })).toBeVisible()

    const downloadPromise = page.waitForEvent('download')
    await page.getByLabel('Export mailbox').click()
    const download = await downloadPromise
    expect(download.suggestedFilename()).toBe('harbor-mail-export.mbox')

    if (!existsSync(resultDir)) mkdirSync(resultDir, { recursive: true })
    await download.saveAs(exportFile)
    const contents = readFileSync(exportFile, 'utf8')
    expect(contents).toContain('From: Nora Li <nora@northstar.studio>')
    expect(contents).toContain('X-Harbor-Id: ')
    expect(contents).toContain('X-Harbor-Folder: ')
    expect(contents).toContain('From alex@harbor.co ')
  })

  test('imports an .mbox file and dedupes on a second import', async ({ page }) => {
    await openInbox(page)
    await page.goto('/mail/all')
    await expect(page.getByRole('heading', { name: 'All Mail', exact: true })).toBeVisible()

    await page.getByTestId('mbox-import').setInputFiles({ name: 'mailbox.mbox', mimeType: 'application/mbox', buffer: Buffer.from(sampleMbox) })
    await expect(page.getByText('Imported 2 messages')).toBeVisible()
    await expect(page.locator('.mail-row', { hasText: 'Imported Standup Notes' })).toBeVisible()

    await page.getByTestId('mbox-import').setInputFiles({ name: 'mailbox.mbox', mimeType: 'application/mbox', buffer: Buffer.from(sampleMbox) })
    await expect(page.getByText('Nothing new to import')).toBeVisible()
    await expect(page.locator('.mail-row').filter({ hasText: 'Imported Design Sync' })).toHaveCount(1)
  })
})