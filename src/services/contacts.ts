import { contacts as seedContacts } from '../contacts'

export type Contact = { name: string; email: string; company?: string; phone?: string }

const contactsKey = 'harbor-mail:contacts'

const cloneSeed = (): Contact[] => seedContacts.map(contact => ({ ...contact }))

export const contactsService = {
  list(): Contact[] {
    try {
      const stored = JSON.parse(localStorage.getItem(contactsKey) || 'null') as Contact[] | null
      return Array.isArray(stored) && stored.length ? stored : cloneSeed()
    } catch {
      return cloneSeed()
    }
  },
  save(contacts: Contact[]) {
    localStorage.setItem(contactsKey, JSON.stringify(contacts))
  },
  add(contact: Contact) {
    const next = [...this.list().filter(existing => existing.email.toLowerCase() !== contact.email.toLowerCase()), contact]
    this.save(next)
    return next
  },
  update(email: string, patch: Partial<Contact>) {
    const next = this.list().map(contact =>
      contact.email.toLowerCase() === email.toLowerCase() ? { ...contact, ...patch, email } : contact,
    )
    this.save(next)
    return next
  },
  upsert(contact: Contact) {
    const match = this.list().find(existing => existing.email.toLowerCase() === contact.email.toLowerCase())
    if (match) return this.update(contact.email, { company: contact.company ?? match.company, phone: contact.phone ?? match.phone })
    return this.add(contact)
  },
  remove(email: string) {
    const next = this.list().filter(contact => contact.email.toLowerCase() !== email.toLowerCase())
    this.save(next)
    return next
  },
}