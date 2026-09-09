import { expect, test } from '@playwright/test'
import { openInbox, openThread } from './helpers'

const pad = (value: number) => String(value).padStart(2, '0')
const todayIcs = () => {
  const now = new Date()
  return [
    'BEGIN:VCALENDAR',
    'VERSION:2.0',
    'PRODID:-//Import Test//EN',
    'BEGIN:VEVENT',
    `DTSTART:${now.getFullYear()}${pad(now.getMonth() + 1)}${pad(now.getDate())}T090000`,
    `DTEND:${now.getFullYear()}${pad(now.getMonth() + 1)}${pad(now.getDate())}T093000`,
    'SUMMARY:Imported standup',
    'END:VEVENT',
    'END:VCALENDAR',
  ].join('\r\n')
}

test.describe('calendar', () => {
  test('opens the calendar from the sidebar and shows the month grid', async ({ page }) => {
    await openInbox(page)
    await page.getByRole('button', { name: 'Calendar', exact: true }).click()
    await expect(page).toHaveURL(/\/mail\/calendar$/)
    await expect(page.getByRole('heading', { name: new RegExp(String(new Date().getFullYear())) })).toBeVisible()
    await expect(page.getByRole('region', { name: 'Month view' })).toBeVisible()
    await expect(page.locator('.calendar-agenda')).toContainText('Team standup')
    await expect(page.locator('.calendar-agenda')).toContainText('Design review')
  })

  test('opens the calendar from the command palette', async ({ page }) => {
    await openInbox(page)
    await page.keyboard.press('Control+k')
    await page.getByRole('combobox', { name: 'Search commands' }).fill('calendar')
    await page.getByRole('option', { name: 'Open calendar' }).click()
    await expect(page).toHaveURL(/\/mail\/calendar$/)
    await expect(page.getByRole('button', { name: 'New event', exact: true })).toBeVisible()
  })

  test('creates, edits and deletes an event', async ({ page }) => {
    await openInbox(page)
    await page.goto('/mail/calendar')
    await page.getByRole('button', { name: 'New event', exact: true }).click()
    const newDialog = page.getByRole('dialog', { name: 'New event' })
    await expect(newDialog).toBeVisible()
    await newDialog.getByLabel('Event title').fill('Board game night')
    await newDialog.getByRole('button', { name: 'Create event' }).click()
    await expect(page.getByText('Event created')).toBeVisible()
    await expect(page.locator('.calendar-agenda')).toContainText('Board game night')

    await page.locator('.calendar-agenda .agenda-row', { hasText: 'Board game night' }).click()
    const editDialog = page.getByRole('dialog', { name: 'Edit event' })
    await expect(editDialog).toBeVisible()
    await editDialog.getByLabel('Event title').fill('Pizza and games')
    await editDialog.getByRole('button', { name: 'Save changes' }).click()
    await expect(page.getByText('Event updated')).toBeVisible()
    await expect(page.locator('.calendar-agenda')).toContainText('Pizza and games')

    await page.locator('.calendar-agenda .agenda-row', { hasText: 'Pizza and games' }).click()
    await page.getByRole('dialog', { name: 'Edit event' }).getByRole('button', { name: 'Delete' }).click()
    await expect(page.getByText('Event deleted')).toBeVisible()
    await expect(page.locator('.calendar-agenda')).not.toContainText('Pizza and games')
  })

  test('exports an event as an .ics file', async ({ page }) => {
    await openInbox(page)
    await page.goto('/mail/calendar')
    await page.locator('.calendar-agenda .agenda-row', { hasText: 'Team standup' }).click()
    const downloadPromise = page.waitForEvent('download')
    await page.getByRole('dialog', { name: 'Edit event' }).getByRole('button', { name: 'Export .ics' }).click()
    const download = await downloadPromise
    expect(download.suggestedFilename()).toMatch(/\.ics$/)
    await page.getByRole('button', { name: 'Close event editor' }).click()
  })

  test('imports events from an .ics file', async ({ page }) => {
    await openInbox(page)
    await page.goto('/mail/calendar')
    await page.locator('input[type="file"]').setInputFiles({ name: 'import.ics', mimeType: 'text/calendar', buffer: Buffer.from(todayIcs()) }, { force: true })
    await expect(page.getByText('Imported 1 event')).toBeVisible()
    await expect(page.locator('.calendar-agenda')).toContainText('Imported standup')
  })

  test('keeps day columns aligned regardless of long event titles', async ({ page }) => {
    await openInbox(page)
    await page.goto('/mail/calendar')
    await page.getByRole('button', { name: 'New event', exact: true }).click()
    const longTitle = 'Quarterly planning offsite review session with the entire product and engineering leadership team'
    const dialog = page.getByRole('dialog', { name: 'New event' })
    await dialog.getByLabel('Event title').fill(longTitle)
    await dialog.getByRole('button', { name: 'Create event' }).click()

    const columns = await page.locator('.calendar-week .calendar-day:nth-child(1)').evaluateAll(nodes =>
      nodes.map(node => {
        const rect = (node as HTMLElement).getBoundingClientRect()
        return { left: rect.left, width: rect.width }
      }),
    )
    expect(columns.length).toBeGreaterThan(1)
    expect(columns.slice(1).every(column => Math.abs(column.left - columns[0].left) < 0.5)).toBe(true)
    expect(columns.slice(1).every(column => Math.abs(column.width - columns[0].width) < 0.5)).toBe(true)
  })

  test('adds an event from an open message', async ({ page }) => {
    await openThread(page, 'Nora Li')
    await page.getByRole('button', { name: 'Add to calendar' }).click()
    await expect(page.getByText('Event added to your calendar')).toBeVisible()
    await page.goto('/mail/calendar')
    await expect(page.locator('.calendar-agenda')).toContainText('Q3 launch plan - review before Thursday')
  })
})
