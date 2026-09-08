import { useCallback, useEffect, useMemo, useState, type ReactNode } from 'react'
import { Bell, CalendarDays, CircleHelp, Search, Server, Settings2, SquarePen, UserRound } from 'lucide-react'
import { folderSlug, folders } from '../lib/mail'

type CommandPaletteProps = {
  open: boolean
  onClose: () => void
  onNavigate: (path: string) => void
  onCompose: () => void
  onOpenSettings: () => void
  onOpenNotifications: () => void
  onOpenContacts: () => void
  onOpenAdmin: () => void
  onOpenCalendar: () => void
  onToggleHelp: () => void
}

type Command = { id: string; label: string; icon: ReactNode; keywords: string; action: () => void }

export function CommandPalette({ open, onClose, onNavigate, onCompose, onOpenSettings, onOpenNotifications, onOpenContacts, onOpenAdmin, onOpenCalendar, onToggleHelp }: CommandPaletteProps) {
  const [query, setQuery] = useState('')
  const [index, setIndex] = useState(0)

  const commands = useMemo<Command[]>(() => {
    const folderCommands: Command[] = folders.map(folder => ({
      id: `folder-${folder.label}`,
      label: `Go to ${folder.label}`,
      icon: <folder.icon key={folder.label} size={15} />,
      keywords: `folder mail navigate ${folder.label}`,
      action: () => onNavigate(`/mail/${folderSlug[folder.label]}`),
    }))
    return [
      ...folderCommands,
      { id: 'calendar', label: 'Open calendar', icon: <CalendarDays size={15} />, keywords: 'calendar schedule event agenda ics', action: onOpenCalendar },
      { id: 'compose', label: 'Compose new message', icon: <SquarePen size={15} />, keywords: 'compose write new email send', action: onCompose },
      { id: 'contacts', label: 'Open contacts', icon: <UserRound size={15} />, keywords: 'contacts people address book', action: onOpenContacts },
      { id: 'notifications', label: 'Tune notifications', icon: <Bell size={15} />, keywords: 'notifications alerts settings', action: onOpenNotifications },
      { id: 'settings', label: 'Open settings', icon: <Settings2 size={15} />, keywords: 'settings preferences options signature', action: onOpenSettings },
      { id: 'admin', label: 'Open admin panel', icon: <Server size={15} />, keywords: 'admin panel mailboxes aliases domain forwarders dns catch-all', action: onOpenAdmin },
      { id: 'help', label: 'Keyboard shortcuts', icon: <CircleHelp size={15} />, keywords: 'keyboard shortcuts help keys guide', action: onToggleHelp },
    ]
  }, [onNavigate, onCompose, onOpenContacts, onOpenNotifications, onOpenSettings, onOpenAdmin, onOpenCalendar, onToggleHelp])

  const filtered = useMemo(() => {
    const terms = query.trim().toLowerCase().split(/\s+/).filter(Boolean)
    return commands.filter(command => terms.every(term => `${command.label} ${command.keywords}`.toLowerCase().includes(term)))
  }, [commands, query])

  const activeIndex = Math.min(index, Math.max(0, filtered.length - 1))
  const close = useCallback(() => { onClose(); setQuery(''); setIndex(0) }, [onClose])

  useEffect(() => {
    if (!open) return
    const handle = (event: KeyboardEvent) => {
      if (event.key === 'Escape') { event.preventDefault(); close() }
      if (event.key === 'ArrowDown') { event.preventDefault(); setIndex(current => (current + 1) % Math.max(1, filtered.length)) }
      if (event.key === 'ArrowUp') { event.preventDefault(); setIndex(current => (current - 1 + Math.max(1, filtered.length)) % Math.max(1, filtered.length)) }
      if (event.key === 'Enter') { event.preventDefault(); const command = filtered[activeIndex]; if (command) { command.action(); close() } }
    }
    window.addEventListener('keydown', handle)
    return () => window.removeEventListener('keydown', handle)
  }, [open, filtered, activeIndex, close])

  if (!open) return null
  return (
    <div className="palette-layer" onClick={close}>
      <section className="palette-panel" role="dialog" aria-modal="true" aria-label="Command palette" onClick={event => event.stopPropagation()}>
        <div className="palette-input">
          <Search size={17} />
          <input value={query} onChange={event => { setQuery(event.target.value); setIndex(0) }} placeholder="Type a command or folder name..." aria-label="Search commands" role="combobox" aria-expanded="true" aria-controls="palette-results" aria-activedescendant={filtered[activeIndex] ? `command-${filtered[activeIndex].id}` : undefined} autoFocus />
          <kbd>Esc</kbd>
        </div>
        <ul className="palette-results" role="listbox" id="palette-results" aria-label="Commands">
          {filtered.map((command, itemIndex) => (
            <li key={command.id} id={`command-${command.id}`} role="option" aria-selected={itemIndex === activeIndex} className={`palette-item ${itemIndex === activeIndex ? 'palette-item--active' : ''}`} onMouseEnter={() => setIndex(itemIndex)} onClick={() => { command.action(); close() }}>
              <span className="palette-item__icon">{command.icon}</span>
              <span>{command.label}</span>
            </li>
          ))}
          {filtered.length === 0 && <li className="palette-empty" role="status">No matches for “{query}”.</li>}
        </ul>
        <footer className="palette-foot"><span><kbd>↑</kbd><kbd>↓</kbd> navigate</span><span><kbd>Enter</kbd> open · <kbd>Esc</kbd> close</span></footer>
      </section>
    </div>
  )
}