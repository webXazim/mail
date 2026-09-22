import { useCallback, useEffect, useMemo, useRef, useState, type FormEvent } from 'react'
import { Download, FileUp, Mail as MailIcon, Pencil, Plus, Search, Trash2 } from 'lucide-react'
import { contactsService, type Contact } from '../services/contacts'
import { useMail } from '../state/mail/MailContext'
import type { RealtimeEvent } from '../services/ws'

const initials = (contact: Contact) =>
  contact.name
    .split(/\s+/)
    .map((part) => part[0])
    .slice(0, 2)
    .join('')
    .toUpperCase()

const emptyForm = { name: '', email: '', company: '', phone: '' }

export function ContactsPage() {
  const { openCompose } = useMail()
  const [list, setList] = useState<Contact[]>(() => contactsService.list())
  const [query, setQuery] = useState('')
  const [editing, setEditing] = useState<Contact | null>(null)
  const [adding, setAdding] = useState(false)
  const [form, setForm] = useState(emptyForm)
  const [error, setError] = useState('')
  const [searchResults, setSearchResults] = useState<Contact[] | null>(null)
  const [total, setTotal] = useState(list.length)
  const [hasMore, setHasMore] = useState(false)
  const [nextCursor, setNextCursor] = useState<string | null>(null)
  const [searchTotal, setSearchTotal] = useState(0)
  const [searchHasMore, setSearchHasMore] = useState(false)
  const [searchCursor, setSearchCursor] = useState<string | null>(null)
  const [loadingMore, setLoadingMore] = useState(false)
  const fileRef = useRef<HTMLInputElement>(null)

  useEffect(() => {
    let cancelled = false
    void contactsService
      .page()
      .then((page) => {
        if (cancelled) return
        contactsService.save(page.contacts)
        setList(page.contacts)
        setTotal(page.total)
        setHasMore(page.hasMore)
        setNextCursor(page.nextCursor)
      })
      .catch((reason: unknown) => {
        if (!cancelled) setError(reason instanceof Error ? reason.message : 'Unable to load contacts')
      })
    return () => {
      cancelled = true
    }
  }, [])

  useEffect(() => {
    const needle = query.trim()
    if (!needle) {
      setSearchResults(null)
      setSearchTotal(0)
      setSearchHasMore(false)
      setSearchCursor(null)
      return
    }
    const timer = window.setTimeout(() => {
      void contactsService
        .page(needle)
        .then((page) => {
          setSearchResults(page.contacts)
          setSearchTotal(page.total)
          setSearchHasMore(page.hasMore)
          setSearchCursor(page.nextCursor)
        })
        .catch((reason: unknown) => setError(reason instanceof Error ? reason.message : 'Search failed'))
    }, 250)
    return () => window.clearTimeout(timer)
  }, [query])

  const filtered = useMemo(() => searchResults ?? list, [list, searchResults])
  const displayTotal = query.trim() ? searchTotal : total
  const displayHasMore = query.trim() ? searchHasMore : hasMore

  const reloadBase = useCallback(async () => {
    const page = await contactsService.page()
    contactsService.save(page.contacts)
    setList(page.contacts)
    setTotal(page.total)
    setHasMore(page.hasMore)
    setNextCursor(page.nextCursor)
  }, [])

  useEffect(() => {
    const onRealtime = (incoming: Event) => {
      const detail = (incoming as CustomEvent<RealtimeEvent>).detail
      if (detail?.kind !== 'resource-changed' || detail.payload.resource !== 'contacts') return
      const needle = query.trim()
      if (!needle) {
        void reloadBase().catch(() => {})
        return
      }
      void contactsService.page(needle).then((page) => {
        setSearchResults(page.contacts)
        setSearchTotal(page.total)
        setSearchHasMore(page.hasMore)
        setSearchCursor(page.nextCursor)
      }).catch(() => {})
    }
    window.addEventListener('cs-mail-realtime', onRealtime)
    return () => window.removeEventListener('cs-mail-realtime', onRealtime)
  }, [query, reloadBase])

  const loadMore = async () => {
    const needle = query.trim()
    const cursor = needle ? searchCursor : nextCursor
    if (!cursor || loadingMore) return
    setLoadingMore(true)
    try {
      const page = await contactsService.page(needle, cursor)
      if (needle) {
        setSearchResults((current) => [...(current ?? []), ...page.contacts])
        setSearchHasMore(page.hasMore)
        setSearchCursor(page.nextCursor)
        setSearchTotal(page.total)
      } else {
        const merged = [...list, ...page.contacts]
        contactsService.save(merged)
        setList(merged)
        setHasMore(page.hasMore)
        setNextCursor(page.nextCursor)
        setTotal(page.total)
      }
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : 'Unable to load more contacts')
    } finally {
      setLoadingMore(false)
    }
  }

  const update = (patch: Partial<typeof emptyForm>) =>
    setForm((current) => ({ ...current, ...patch }))

  const startAdd = () => {
    setEditing(null)
    setForm(emptyForm)
    setAdding(true)
    setError('')
  }

  const startEdit = (contact: Contact) => {
    setAdding(false)
    setEditing(contact)
    setForm({
      name: contact.name,
      email: contact.email,
      company: contact.company ?? '',
      phone: contact.phone ?? '',
    })
    setError('')
  }

  const cancel = () => {
    setAdding(false)
    setEditing(null)
    setForm(emptyForm)
    setError('')
  }

  const save = async (event: FormEvent) => {
    event.preventDefault()
    const email = form.email.trim().toLowerCase()
    if (!/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email)) {
      setError('Enter a valid email address')
      return
    }
    const contact: Contact = {
      name: form.name.trim() || email.split('@')[0],
      email,
      company: form.company.trim(),
      phone: form.phone.trim(),
    }
    try {
      if (editing) await contactsService.update(editing.email, contact)
      else await contactsService.add(contact)
      setQuery('')
      setSearchResults(null)
      await reloadBase()
      cancel()
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : 'Unable to save contact')
    }
  }

  const remove = (contact: Contact) => {
    setError('')
    void contactsService
      .remove(contact.email)
      .then(() => {
        setQuery('')
        setSearchResults(null)
        return reloadBase()
      })
      .catch((reason: unknown) => setError(reason instanceof Error ? reason.message : 'Unable to remove contact'))
  }

  const exportContacts = () => {
    setError('')
    void contactsService
      .exportCsv()
      .then(({ filename, content }) => {
        const blob = new Blob([content], { type: 'text/csv;charset=utf-8' })
        const url = URL.createObjectURL(blob)
        const link = document.createElement('a')
        link.href = url
        link.download = filename
        link.click()
        URL.revokeObjectURL(url)
      })
      .catch((reason: unknown) => setError(reason instanceof Error ? reason.message : 'Unable to export contacts'))
  }

  const importContacts = (file: File) => {
    setError('')
    void file
      .text()
      .then((content) => contactsService.importCsv(content, true))
      .then(() => {
        setQuery('')
        setSearchResults(null)
        return reloadBase()
      })
      .catch((reason: unknown) => setError(reason instanceof Error ? reason.message : 'Unable to import contacts'))
  }

  return (
    <div className="contacts-page">
      <header className="contacts-head">
        <div>
          <p className="eyebrow">Address book</p>
          <h1>Contacts</h1>
          <p className="contacts-head__count">{total} live contacts</p>
        </div>
        <div className="row-actions">
          <input
            ref={fileRef}
            type="file"
            accept=".csv,text/csv"
            hidden
            aria-label="Import contacts CSV"
            onChange={(event) => {
              const file = event.target.files?.[0]
              if (file) importContacts(file)
              event.currentTarget.value = ''
            }}
          />
          <button type="button" className="secondary-button" onClick={() => fileRef.current?.click()}>
            <FileUp size={15} />
            Import
          </button>
          <button type="button" className="secondary-button" onClick={exportContacts}>
            <Download size={15} />
            Export
          </button>
          <button type="button" className="primary-button" onClick={startAdd}>
            <Plus size={15} />
            New contact
          </button>
        </div>
      </header>

      <div className="contacts-body">
        <div className="contacts-query">
          <Search size={16} />
          <input
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder="Search by name, email, company or phone"
            aria-label="Search contacts"
          />
        </div>
        {query.trim() && <p className="settings-hint">{displayTotal} matching contact{displayTotal === 1 ? '' : 's'}</p>}
        {error && !adding && !editing && <p className="composer-error">{error}</p>}

        {(adding || editing) && (
          <form className="contacts-form" onSubmit={save}>
            <h2>{editing ? `Edit ${editing.name}` : 'New contact'}</h2>
            <label>
              Name
              <input
                aria-label="Contact name"
                value={form.name}
                onChange={(event) => update({ name: event.target.value })}
                placeholder="Jane Appleseed"
              />
            </label>
            <label>
              Email
              <input
                type="email"
                aria-label="Contact email"
                value={form.email}
                onChange={(event) => update({ email: event.target.value })}
                placeholder="jane@appleseed.example"
                required
              />
            </label>
            <label>
              Company
              <input
                aria-label="Contact company"
                value={form.company}
                onChange={(event) => update({ company: event.target.value })}
                placeholder="Acme Inc."
              />
            </label>
            <label>
              Phone
              <input
                aria-label="Contact phone"
                value={form.phone}
                onChange={(event) => update({ phone: event.target.value })}
                placeholder="+1 (555) 010-0000"
              />
            </label>
            {error && <p className="composer-error">{error}</p>}
            <footer>
              <button type="button" className="secondary-button" onClick={cancel}>
                Cancel
              </button>
              <button type="submit" className="primary-button">
                {editing ? (
                  <>
                    <Pencil size={15} />
                    Save changes
                  </>
                ) : (
                  <>
                    <Plus size={15} />
                    Add contact
                  </>
                )}
              </button>
            </footer>
          </form>
        )}

        <ul className="contacts-list">
          {filtered.map((contact, index) => (
            <li key={contact.email}>
              <span
                className={`avatar avatar--${['coral', 'teal', 'purple', 'orange', 'blue', 'green'][index % 6]}`}
              >
                {initials(contact)}
              </span>
              <span className="contacts-list__text">
                <strong>{contact.name}</strong>
                <small>
                  {contact.email}
                  {contact.company ? ` · ${contact.company}` : ''}
                  {contact.phone ? ` · ${contact.phone}` : ''}
                </small>
              </span>
              <button
                type="button"
                className="icon-button"
                aria-label={`Email ${contact.name}`}
                title="Compose to this contact"
                onClick={() => openCompose({ to: `${contact.name} <${contact.email}>` })}
              >
                <MailIcon size={15} />
              </button>
              <button
                type="button"
                className="icon-button"
                aria-label={`Edit ${contact.name}`}
                title="Edit contact"
                onClick={() => startEdit(contact)}
              >
                <Pencil size={15} />
              </button>
              <button
                type="button"
                className="icon-button"
                aria-label={`Remove ${contact.name}`}
                title="Remove contact"
                onClick={() => remove(contact)}
              >
                <Trash2 size={15} />
              </button>
            </li>
          ))}
          {filtered.length === 0 && (
            <li className="list-state">
              {query ? (
                <>No contacts match “{query}”.</>
              ) : (
                <>Your address book is empty — add a contact.</>
              )}
            </li>
          )}
        </ul>
        {displayHasMore && (
          <div className="row-actions">
            <button type="button" className="secondary-button" onClick={() => void loadMore()} disabled={loadingMore}>
              {loadingMore ? 'Loading…' : 'Load more'}
            </button>
          </div>
        )}
      </div>
    </div>
  )
}
