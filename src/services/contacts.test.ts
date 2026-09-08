import { beforeEach, describe, expect, it } from 'vitest'
import { contactsService } from './contacts'

describe('contactsService', () => {
  beforeEach(() => localStorage.clear())

  const seedNames = () => contactsService.list().map(contact => contact.name).sort()

  it('falls back to the seeded address book', () => {
    expect(seedNames()).toEqual(['Alex Chen', 'Jonas Meier', 'Nora Li', 'Priya Shah', 'Riley Brooks'])
    expect(contactsService.list()[0]).toHaveProperty('company')
  })

  it('adds a contact and de-dupes by email', () => {
    const added = contactsService.add({ name: 'Ada Lovelace', email: 'ada@analytical.example' })
    expect(added).toHaveLength(6)
    const again = contactsService.add({ name: 'Ada', email: 'ada@analytical.example' })
    expect(again).toHaveLength(6)
    expect(again.find(contact => contact.email === 'ada@analytical.example')?.name).toBe('Ada')
  })

  it('updates a contact by email without moving it', () => {
    const next = contactsService.update('nora@northstar.studio', { name: 'Nora Li', company: 'Northstar Studio · SF' })
    const updated = next.find(contact => contact.email === 'nora@northstar.studio')
    expect(updated?.company).toBe('Northstar Studio · SF')
    expect(updated?.phone).toBe('+1 (415) 555-0134')
    expect(next.length).toBe(5)
  })

  it('upserts new senders and preserves existing contact details', () => {
    contactsService.upsert({ name: 'new.friend', email: 'friend@fold.example' })
    expect(contactsService.list()).toHaveLength(6)
    contactsService.upsert({ name: 'new.friend', email: 'friend@fold.example' })
    expect(contactsService.list()).toHaveLength(6)
    const known = contactsService.upsert({ name: 'priya', email: 'priya@harbor.co' })
    expect(known.find(contact => contact.email === 'priya@harbor.co')?.name).toBe('Priya Shah')
  })

  it('removes a contact by email', () => {
    expect(contactsService.remove('jonas@meridian.co')).toHaveLength(4)
  })
})