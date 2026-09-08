import { contacts as seedContacts } from '../contacts'

export type Contact = { name: string; email: string }

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
  remove(email: string) {
    const next = this.list().filter(contact => contact.email.toLowerCase() !== email.toLowerCase())
    this.save(next)
    return next
  },
}