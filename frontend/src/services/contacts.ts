import { contacts as seedContacts } from '../contacts'
import { ApiError, apiFetch } from '../lib/api'
import { isRemoteMail } from './remote-mail'

export type Contact = {
  id?: string
  name: string
  email: string
  company?: string
  phone?: string
  version?: number
  updatedAt?: string
}

type ContactRow = {
  id: string
  name: string
  email: string
  company: string
  phone: string
  version: number
  updatedAt: string
}

type ContactPage = {
  contacts: ContactRow[]
  total: number
  hasMore: boolean
  nextCursor: string | null
}

export type ContactPageResult = {
  contacts: Contact[]
  total: number
  hasMore: boolean
  nextCursor: string | null
}

const contactsKey = 'cs-mail:contacts'
const cloneSeed = (): Contact[] => seedContacts.map((contact) => ({ ...contact }))

const rowToContact = (row: ContactRow): Contact => ({
  id: row.id,
  name: row.name,
  email: row.email,
  company: row.company || undefined,
  phone: row.phone || undefined,
  version: row.version,
  updatedAt: row.updatedAt,
})

const readCache = (): Contact[] | null => {
  try {
    const stored = JSON.parse(localStorage.getItem(contactsKey) || 'null') as Contact[] | null
    return Array.isArray(stored) ? stored : null
  } catch {
    return null
  }
}

const matches = (contact: Contact, email: string) =>
  contact.email.toLowerCase() === email.toLowerCase()

const saveCache = (contacts: Contact[]) => {
  localStorage.setItem(contactsKey, JSON.stringify(contacts))
}

const remotePage = async (q = '', cursor?: string | null): Promise<ContactPageResult> => {
  const params = new URLSearchParams({ limit: '100' })
  if (q.trim()) params.set('q', q.trim())
  if (cursor) params.set('cursor', cursor)
  const page = await apiFetch<ContactPage>(`/api/contacts?${params}`)
  return {
    contacts: (page.contacts ?? []).map(rowToContact),
    total: page.total ?? 0,
    hasMore: Boolean(page.hasMore),
    nextCursor: page.nextCursor ?? null,
  }
}

export const contactsService = {
  /** Local cache is presentation-only in authenticated mode and authoritative only in demo mode. */
  list(): Contact[] {
    if (isRemoteMail()) return readCache() ?? []
    return readCache() ?? cloneSeed()
  },
  save(contacts: Contact[]) {
    saveCache(contacts)
  },
  async page(query = '', cursor?: string | null): Promise<ContactPageResult> {
    if (isRemoteMail()) return remotePage(query, cursor)
    const needle = query.trim().toLowerCase()
    const all = needle
      ? this.list().filter((contact) =>
          `${contact.name} ${contact.email} ${contact.company ?? ''} ${contact.phone ?? ''}`
            .toLowerCase()
            .includes(needle),
        )
      : this.list()
    return { contacts: all, total: all.length, hasMore: false, nextCursor: null }
  },
  async refresh(): Promise<Contact[]> {
    const page = await this.page()
    if (isRemoteMail()) saveCache(page.contacts)
    return page.contacts
  },
  async search(query: string): Promise<Contact[]> {
    return (await this.page(query)).contacts
  },
  async add(contact: Contact): Promise<Contact[]> {
    if (!isRemoteMail()) {
      const next = [...this.list().filter((existing) => !matches(existing, contact.email)), contact]
      saveCache(next)
      return next
    }
    const row = await apiFetch<ContactRow>('/api/contacts', {
      method: 'POST',
      body: JSON.stringify(contact),
    })
    const next = [
      ...this.list().filter((existing) => !matches(existing, row.email)),
      rowToContact(row),
    ]
    saveCache(next)
    return next
  },
  async update(email: string, patch: Partial<Contact>): Promise<Contact[]> {
    const current = this.list()
    let match = current.find((contact) => matches(contact, email))
    if (!isRemoteMail()) {
      const next = current.map((contact) =>
        matches(contact, email) ? { ...contact, ...patch, email: patch.email ?? email } : contact,
      )
      saveCache(next)
      return next
    }
    if (!match?.id || !match.version) {
      const page = await remotePage(email)
      match = page.contacts.find((contact) => matches(contact, email))
    }
    if (!match?.id || !match.version) throw new Error('Refresh this contact before editing it.')
    const row = await apiFetch<ContactRow>(`/api/contacts/${encodeURIComponent(match.id)}`, {
      method: 'PUT',
      body: JSON.stringify({ ...patch, email: patch.email ?? email, version: match.version }),
    })
    const next = current.map((contact) =>
      matches(contact, email) ? rowToContact(row) : contact,
    )
    saveCache(next)
    return next
  },
  async upsert(contact: Contact): Promise<Contact[]> {
    const existing = this.list().find((item) => matches(item, contact.email))
    if (existing) {
      return this.update(existing.email, {
        name: contact.name || existing.name,
        company: contact.company ?? existing.company,
        phone: contact.phone ?? existing.phone,
      })
    }
    if (!isRemoteMail()) return this.add(contact)
    try {
      return await this.add(contact)
    } catch (reason) {
      if (!(reason instanceof ApiError) || reason.status !== 409) throw reason
      const page = await remotePage(contact.email)
      const match = page.contacts.find((item) => matches(item, contact.email))
      if (!match?.id || !match.version) throw reason
      const row = await apiFetch<ContactRow>(`/api/contacts/${encodeURIComponent(match.id)}`, {
        method: 'PUT',
        body: JSON.stringify({
          name: contact.name || match.name,
          email: contact.email,
          company: contact.company ?? match.company,
          phone: contact.phone ?? match.phone,
          version: match.version,
        }),
      })
      const next = [
        ...this.list().filter((item) => !matches(item, contact.email)),
        rowToContact(row),
      ]
      saveCache(next)
      return next
    }
  },
  async remove(email: string): Promise<Contact[]> {
    const current = this.list()
    let match = current.find((contact) => matches(contact, email))
    if (isRemoteMail()) {
      if (!match?.id) {
        const page = await remotePage(email)
        match = page.contacts.find((contact) => matches(contact, email))
      }
      if (!match?.id || !match.version) throw new Error('Refresh this contact before deleting it.')
      await apiFetch(`/api/contacts/${encodeURIComponent(match.id)}?version=${encodeURIComponent(String(match.version))}`, { method: 'DELETE' })
    }
    const next = current.filter((contact) => !matches(contact, email))
    saveCache(next)
    return next
  },
  async exportCsv(): Promise<{ filename: string; content: string }> {
    if (!isRemoteMail()) {
      const escape = (value: string) => {
        if (!/[",\r\n]/.test(value)) return value
        return `"${value.replace(/"/g, '""')}"`
      }
      const lines = ['name,email,company,phone']
      for (const item of this.list()) {
        lines.push(
          [item.name, item.email, item.company ?? '', item.phone ?? ''].map(escape).join(','),
        )
      }
      return { filename: 'cs-mail-contacts.csv', content: `${lines.join('\r\n')}\r\n` }
    }
    return apiFetch<{ filename: string; content: string }>('/api/contacts/export')
  },
  async importCsv(content: string, replaceExisting = true): Promise<Contact[]> {
    if (!isRemoteMail()) throw new Error('Contact import requires an authenticated mailbox.')
    await apiFetch('/api/contacts/import', {
      method: 'POST',
      body: JSON.stringify({ content, replaceExisting }),
    })
    return this.refresh()
  },
}
