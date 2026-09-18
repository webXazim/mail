import { contacts as seedContacts } from '../contacts'
import { apiFetch } from '../lib/api'
import { isRemoteMail } from './remote-mail'

export type Contact = {
  id?: string
  name: string
  email: string
  company?: string
  phone?: string
}

type ContactRow = {
  id: string
  name: string
  email: string
  company: string
  phone: string
}

const contactsKey = 'harbor-mail:contacts'

const cloneSeed = (): Contact[] => seedContacts.map((contact) => ({ ...contact }))

const rowToContact = (row: ContactRow): Contact => ({
  id: row.id,
  name: row.name,
  email: row.email,
  company: row.company || undefined,
  phone: row.phone || undefined,
})

const readCache = (): Contact[] | null => {
  try {
    const stored = JSON.parse(localStorage.getItem(contactsKey) || 'null') as Contact[] | null
    return Array.isArray(stored) && stored.length ? stored : null
  } catch {
    return null
  }
}

const matches = (contact: Contact, email: string) =>
  contact.email.toLowerCase() === email.toLowerCase()

export const contactsService = {
  /** Synchronous read from the local cache (seeded in demo mode). */
  list(): Contact[] {
    return readCache() ?? cloneSeed()
  },
  save(contacts: Contact[]) {
    localStorage.setItem(contactsKey, JSON.stringify(contacts))
  },
  /** API-first refresh; falls back to the local cache when offline or in demo mode. */
  async refresh(): Promise<Contact[]> {
    if (!isRemoteMail()) return this.list()
    try {
      const result = await apiFetch<{ contacts: ContactRow[] }>('/api/contacts')
      const next = (result.contacts ?? []).map(rowToContact)
      this.save(next)
      return next
    } catch {
      return this.list()
    }
  },
  async add(contact: Contact): Promise<Contact[]> {
    const localNext = [
      ...this.list().filter((existing) => !matches(existing, contact.email)),
      contact,
    ]
    if (isRemoteMail()) {
      try {
        const row = await apiFetch<ContactRow>('/api/contacts', {
          method: 'POST',
          body: JSON.stringify(contact),
        })
        const next = [
          ...this.list().filter((existing) => !matches(existing, row.email)),
          rowToContact(row),
        ]
        this.save(next)
        return next
      } catch {
        // Offline: keep the optimistic local list.
      }
    }
    this.save(localNext)
    return localNext
  },
  async update(email: string, patch: Partial<Contact>): Promise<Contact[]> {
    const current = this.list()
    const match = current.find((contact) => matches(contact, email))
    if (isRemoteMail() && match?.id) {
      try {
        const row = await apiFetch<ContactRow>(`/api/contacts/${encodeURIComponent(match.id)}`, {
          method: 'PUT',
          body: JSON.stringify({ ...patch, email: patch.email ?? email }),
        })
        const next = current.map((contact) =>
          matches(contact, email) ? rowToContact(row) : contact,
        )
        this.save(next)
        return next
      } catch {
        // Fall through to the local update so the UI stays responsive offline.
      }
    }
    const next = current.map((contact) =>
      matches(contact, email) ? { ...contact, ...patch, email } : contact,
    )
    this.save(next)
    return next
  },
  async upsert(contact: Contact): Promise<Contact[]> {
    const match = this.list().find((existing) => matches(existing, contact.email))
    if (match)
      return this.update(contact.email, {
        company: contact.company ?? match.company,
        phone: contact.phone ?? match.phone,
      })
    if (isRemoteMail()) {
      try {
        const row = await apiFetch<ContactRow>('/api/contacts', {
          method: 'POST',
          body: JSON.stringify(contact),
        })
        const next = [
          ...this.list().filter((existing) => !matches(existing, row.email)),
          rowToContact(row),
        ]
        this.save(next)
        return next
      } catch {
        // Already exists server-side (or offline): leave the cache untouched.
        return this.list()
      }
    }
    return this.add(contact)
  },
  async remove(email: string): Promise<Contact[]> {
    const current = this.list()
    const match = current.find((contact) => matches(contact, email))
    if (isRemoteMail() && match?.id) {
      try {
        await apiFetch(`/api/contacts/${encodeURIComponent(match.id)}`, { method: 'DELETE' })
      } catch {
        // Fall through to the local removal so the UI stays responsive offline.
      }
    }
    const next = current.filter((contact) => !matches(contact, email))
    this.save(next)
    return next
  },
}
