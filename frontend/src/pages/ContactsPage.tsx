import { useEffect, useMemo, useState, type FormEvent } from 'react'
import { Mail as MailIcon, Pencil, Plus, Search, Trash2 } from 'lucide-react'
import { contactsService, type Contact } from '../services/contacts'
import { useMail } from '../state/mail/MailContext'

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

  useEffect(() => {
    let cancelled = false
    void (async () => {
      const rows = await contactsService.refresh()
      if (!cancelled) setList(rows)
    })()
    return () => {
      cancelled = true
    }
  }, [])

  const filtered = useMemo(() => {
    const needle = query.trim().toLowerCase()
    if (!needle) return list
    return list.filter((contact) =>
      `${contact.name} ${contact.email} ${contact.company ?? ''} ${contact.phone ?? ''}`
        .toLowerCase()
        .includes(needle),
    )
  }, [list, query])

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
      company: form.company.trim() || undefined,
      phone: form.phone.trim() || undefined,
    }
    const next = editing
      ? await contactsService.update(editing.email, contact)
      : await contactsService.add(contact)
    setList(next)
    cancel()
  }

  const remove = (contact: Contact) => {
    void contactsService.remove(contact.email).then(setList)
  }

  return (
    <div className="contacts-page">
      <header className="contacts-head">
        <div>
          <p className="eyebrow">Address book</p>
          <h1>Contacts</h1>
          <p className="contacts-head__count">{list.length} live contacts</p>
        </div>
        <button type="button" className="primary-button" onClick={startAdd}>
          <Plus size={15} />
          New contact
        </button>
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
          {filtered.map((contact) => (
            <li key={contact.email}>
              <span
                className={`avatar avatar--${['coral', 'teal', 'purple', 'orange', 'blue', 'green'][list.indexOf(contact) % 6]}`}
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
      </div>
    </div>
  )
}
