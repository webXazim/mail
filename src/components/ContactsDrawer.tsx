import { useRef, useState, type FormEvent } from 'react'
import { Plus, Search, Trash2, X } from 'lucide-react'
import { useFocusTrap } from '../hooks/useFocusTrap'
import { contactsService, type Contact } from '../services/contacts'

export function ContactsDrawer({ close }: { close: () => void }) {
  const [list, setList] = useState<Contact[]>(() => contactsService.list())
  const [name, setName] = useState('')
  const [email, setEmail] = useState('')
  const [query, setQuery] = useState('')
  const panelRef = useRef<HTMLElement>(null)
  useFocusTrap(panelRef)
  const filtered = list.filter(contact => `${contact.name} ${contact.email}`.toLowerCase().includes(query.toLowerCase()))
  const initials = (contact: Contact) => contact.name.split(/\s+/).map(part => part[0]).slice(0, 2).join('').toUpperCase()
  const add = (event: FormEvent) => {
    event.preventDefault()
    const target = email.trim()
    if (!target.includes('@')) return
    setList(contactsService.add({ name: name.trim() || target.split('@')[0], email: target }))
    setName('')
    setEmail('')
  }
  const remove = (contactEmail: string) => setList(contactsService.remove(contactEmail))
  return (
    <div className="settings-layer" role="presentation">
      <section ref={panelRef} className="settings-panel" role="dialog" aria-modal="true" aria-label="Contacts">
        <header>
          <div><p className="eyebrow">Harbor Mail</p><h2>Contacts</h2></div>
          <button type="button" className="icon-button" aria-label="Close contacts" onClick={close}><X size={17} /></button>
        </header>
        <div className="contacts-query"><Search size={16} /><input value={query} onChange={event => setQuery(event.target.value)} placeholder="Find a contact" aria-label="Search contacts" /></div>
        <form className="contacts-add" onSubmit={add}>
          <input value={name} onChange={event => setName(event.target.value)} placeholder="Name" aria-label="Contact name" />
          <input type="email" value={email} onChange={event => setEmail(event.target.value)} placeholder="Email" aria-label="Contact email" />
          <button className="primary-button" type="submit"><Plus size={15} />Add</button>
        </form>
        <ul className="contacts-list">
          {filtered.map(contact => (
            <li key={contact.email}>
              <span className={`avatar avatar--coral`}>{initials(contact)}</span>
              <span className="contacts-list__text"><strong>{contact.name}</strong><small>{contact.email}</small></span>
              <button className="icon-button" aria-label={`Remove ${contact.name}`} onClick={() => remove(contact.email)}><Trash2 size={15} /></button>
            </li>
          ))}
          {filtered.length === 0 && <li className="list-state">No contacts match “{query}”.</li>}
        </ul>
      </section>
    </div>
  )
}