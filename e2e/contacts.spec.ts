import { expect, test } from '@playwright/test'
import { openInbox } from './helpers'

test.describe('contacts', () => {
  test('lists deep contacts and searches by company', async ({ page }) => {
    await openInbox(page)
    await page.goto('/mail/contacts')
    await expect(page.getByRole('heading', { name: 'Contacts', exact: true })).toBeVisible()
    await expect(page.getByText('5 live contacts')).toBeVisible()
    await expect(page.getByText('Nora Li')).toBeVisible()
    await expect(page.getByText('nora@northstar.studio · Northstar Studio · +1 (415) 555-0134')).toBeVisible()
    await page.getByLabel('Search contacts').fill('Northstar')
    await expect(page.locator('.contacts-list li')).toHaveCount(1)
    await expect(page.getByText('Nora Li')).toBeVisible()
  })

  test('adds, edits and removes a contact, then composes from the row', async ({ page }) => {
    await openInbox(page)
    await page.goto('/mail/contacts')
    await page.getByRole('button', { name: 'New contact' }).click()
    await page.getByLabel('Contact name').fill('Grace Hopper')
    await page.getByLabel('Contact email').fill('grace@navy.mil')
    await page.getByLabel('Contact company').fill('US Navy')
    await page.getByRole('button', { name: 'Add contact' }).click()
    await expect(page.getByText('6 live contacts')).toBeVisible()
    await expect(page.getByText('grace@navy.mil · US Navy')).toBeVisible()

    await page.getByLabel('Edit Grace Hopper').click()
    await page.getByLabel('Contact company').fill('United States Navy')
    await page.getByRole('button', { name: 'Save changes' }).click()
    await expect(page.getByText('grace@navy.mil · United States Navy')).toBeVisible()

    await page.getByLabel('Email Grace Hopper').click()
    await expect(page.getByRole('dialog', { name: 'New message' })).toBeVisible()
    await expect(page.getByPlaceholder('Recipients')).toHaveValue('Grace Hopper <grace@navy.mil>')
    await page.getByPlaceholder('Subject').fill('New contact mails')
    await page.getByRole('button', { name: 'Send' }).click()
    await expect(page.getByRole('status').filter({ hasText: 'Message sent' })).toBeVisible()

    await page.goto('/mail/contacts')
    await page.getByLabel('Remove Grace Hopper').click()
    await expect(page.getByText('5 live contacts')).toBeVisible()
    await expect(page.getByText('grace@navy.mil')).toHaveCount(0)
  })
})