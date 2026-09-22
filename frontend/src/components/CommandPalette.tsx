import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import {
  Bell,
  CalendarDays,
  CircleHelp,
  Clock,
  History,
  LayoutTemplate,
  LifeBuoy,
  Search,
  Server,
  Settings2,
  SquarePen,
  UserRound,
} from 'lucide-react'
import { folderSlug, folders } from '../lib/mail'
import { useFocusTrap } from '../hooks/useFocusTrap'
import { useRole } from '../services/profile'
import { isLocalAdminOrigin } from '../lib/admin-origin'

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

type Command = {
  id: string
  label: string
  description?: string
  icon: ReactNode
  keywords: string
  section: string
  action: () => void
}

const recentKey = 'cs-mail:recent-searches'
const loadRecent = (): string[] => {
  try {
    return JSON.parse(localStorage.getItem(recentKey) || '[]')
  } catch {
    return []
  }
}
const saveRecent = (q: string) => {
  const next = loadRecent()
    .filter((s) => s !== q)
    .slice(0, 5)
  localStorage.setItem(recentKey, JSON.stringify([q, ...next]))
}

export function CommandPalette({
  open,
  onClose,
  onNavigate,
  onCompose,
  onOpenSettings,
  onOpenNotifications,
  onOpenContacts,
  onOpenAdmin,
  onOpenCalendar,
  onToggleHelp,
}: CommandPaletteProps) {
  const [query, setQuery] = useState('')
  const [index, setIndex] = useState(0)
  const role = useRole()
  const panelRef = useRef<HTMLElement | null>(null)
  useFocusTrap(panelRef, open)

  const commands = useMemo<Command[]>(() => {
    const folderCommands: Command[] = folders.map((folder) => ({
      id: `folder-${folder.label}`,
      label: `Go to ${folder.label}`,
      icon: <folder.icon key={folder.label} size={15} />,
      keywords: `folder mail navigate ${folder.label}`,
      section: 'Folders',
      action: () => onNavigate(`/mail/${folderSlug[folder.label]}`),
    }))
    return [
      {
        id: 'compose',
        label: 'Compose new message',
        icon: <SquarePen size={15} />,
        keywords: 'compose write new email send',
        section: 'Actions',
        action: onCompose,
      },
      {
        id: 'calendar',
        label: 'Open calendar',
        icon: <CalendarDays size={15} />,
        keywords: 'calendar schedule event agenda ics',
        section: 'Actions',
        action: onOpenCalendar,
      },
      {
        id: 'contacts',
        label: 'Open contacts',
        icon: <UserRound size={15} />,
        keywords: 'contacts people address book',
        section: 'Actions',
        action: onOpenContacts,
      },
      {
        id: 'templates',
        label: 'Open templates',
        icon: <LayoutTemplate size={15} />,
        keywords: 'templates canned responses follow up insert',
        section: 'Actions',
        action: () => onNavigate('/mail/templates'),
      },
      {
        id: 'notifications',
        label: 'Open notifications',
        icon: <Bell size={15} />,
        keywords: 'notifications alerts bell center',
        section: 'Actions',
        action: onOpenNotifications,
      },
      {
        id: 'settings',
        label: 'Open settings',
        icon: <Settings2 size={15} />,
        keywords: 'settings preferences options signature',
        section: 'Actions',
        action: onOpenSettings,
      },
      ...(role === 'admin' && isLocalAdminOrigin()
        ? [
            {
              id: 'admin',
              label: 'Open admin panel',
              icon: <Server size={15} />,
              keywords: 'admin panel mailboxes aliases domain forwarders dns catch-all',
              section: 'Actions',
              action: onOpenAdmin,
            },
          ]
        : []),
      {
        id: 'audit',
        label: 'Open audit log',
        icon: <History size={15} />,
        keywords: 'audit log activity security history billing',
        section: 'Actions',
        action: () => onNavigate('/mail/audit-log'),
      },
      {
        id: 'help',
        label: 'Open help center',
        icon: <CircleHelp size={15} />,
        keywords: 'help center faq guide support articles',
        section: 'Actions',
        action: () => onNavigate('/help'),
      },
      {
        id: 'contact',
        label: 'Contact support',
        icon: <LifeBuoy size={15} />,
        keywords: 'contact support message human',
        section: 'Actions',
        action: () => onNavigate('/contact'),
      },
      {
        id: 'status',
        label: 'Check service status',
        icon: <Clock size={15} />,
        keywords: 'status page outage incident uptime operational',
        section: 'Actions',
        action: () => onNavigate('/status'),
      },
      {
        id: 'shortcuts',
        label: 'Keyboard shortcuts',
        icon: <CircleHelp size={15} />,
        keywords: 'keyboard shortcuts help keys guide',
        section: 'Actions',
        action: onToggleHelp,
      },
      ...folderCommands,
    ]
  }, [
    onNavigate,
    onCompose,
    onOpenContacts,
    onOpenNotifications,
    onOpenSettings,
    onOpenAdmin,
    onOpenCalendar,
    onToggleHelp,
    role,
  ])

  const recent = useMemo(() => (open && !query.trim() ? loadRecent() : []), [open, query])

  const searchCommand = (value: string, navigate: (path: string) => void): Command => ({
    id: 'search',
    label: `Search mail for "${value}"`,
    description: 'Search',
    icon: <Search size={15} />,
    keywords: 'search find query mail messages',
    section: 'Search',
    action: () => navigate(`/mail/search?q=${encodeURIComponent(value)}`),
  })

  const filtered = useMemo(() => {
    const trimmed = query.trim().toLowerCase()
    const terms = trimmed.split(/\s+/).filter(Boolean)
    const isSearchLike = /(^|\s)(from|to|subject|label|in|is|has):/.test(trimmed)
    const matched = terms.length
      ? commands.filter((command) =>
          terms.every((term) =>
            `${command.label} ${command.keywords}`.toLowerCase().includes(term),
          ),
        )
      : commands
    const results: Command[] = []
    if (!trimmed) {
      results.push(...matched)
      return results
    }
    if (isSearchLike) {
      results.push(searchCommand(query.trim(), onNavigate))
    }
    results.push(...matched)
    if (!isSearchLike && matched.length === 0) {
      results.push(searchCommand(query.trim(), onNavigate))
    }
    return results
  }, [commands, query, onNavigate])

  const sections = useMemo(() => {
    const seen = new Set<string>()
    return filtered.reduce<string[]>((acc, command) => {
      if (!seen.has(command.section)) {
        seen.add(command.section)
        acc.push(command.section)
      }
      return acc
    }, [])
  }, [filtered])

  const activeIndex = Math.min(index, Math.max(0, filtered.length - 1))
  const close = useCallback(() => {
    onClose()
    setQuery('')
    setIndex(0)
  }, [onClose])

  useEffect(() => {
    if (!open) return
    const handle = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        event.preventDefault()
        close()
      }
      if (event.key === 'ArrowDown') {
        event.preventDefault()
        setIndex((current) => (current + 1) % Math.max(1, filtered.length))
      }
      if (event.key === 'ArrowUp') {
        event.preventDefault()
        setIndex(
          (current) => (current - 1 + Math.max(1, filtered.length)) % Math.max(1, filtered.length),
        )
      }
      if (event.key === 'Enter') {
        event.preventDefault()
        const command = filtered[activeIndex]
        if (command) {
          if (command.id === 'search' && query.trim()) saveRecent(query.trim())
          command.action()
          close()
        }
      }
    }
    window.addEventListener('keydown', handle)
    return () => window.removeEventListener('keydown', handle)
  }, [open, filtered, activeIndex, close, query])

  if (!open) return null
  return (
    <div className="palette-layer" onClick={close}>
      <section
        ref={panelRef}
        className="palette-panel"
        role="dialog"
        aria-modal="true"
        aria-label="Command palette"
        onClick={(event) => event.stopPropagation()}
      >
        <div className="palette-input">
          <Search size={17} />
          <input
            value={query}
            onChange={(event) => {
              setQuery(event.target.value)
              setIndex(0)
            }}
            placeholder="Type a command, folder, or search..."
            aria-label="Search commands"
            role="combobox"
            aria-expanded="true"
            aria-controls="palette-results"
            aria-activedescendant={
              filtered[activeIndex] ? `command-${filtered[activeIndex].id}` : undefined
            }
          />
          <kbd>Esc</kbd>
        </div>
        <ul className="palette-results" role="listbox" id="palette-results" aria-label="Commands">
          {!query.trim() && recent.length > 0 && (
            <li className="palette-section" aria-hidden="true">
              Recent searches
            </li>
          )}
          {!query.trim() &&
            recent.map((entry) => (
              <li
                key={`recent-${entry}`}
                role="option"
                className="palette-item"
                onClick={() => {
                  saveRecent(entry)
                  onNavigate(`/mail/search?q=${encodeURIComponent(entry)}`)
                  close()
                }}
              >
                <span className="palette-item__icon">
                  <Clock size={15} />
                </span>
                <span>{entry}</span>
              </li>
            ))}
          {sections.map((section) => (
            <li key={`section-${section}`} className="palette-section" aria-hidden="true">
              {section}
            </li>
          ))}
          {filtered.map((command, itemIndex) => (
            <li
              key={command.id}
              id={`command-${command.id}`}
              role="option"
              aria-selected={itemIndex === activeIndex}
              className={`palette-item ${itemIndex === activeIndex ? 'palette-item--active' : ''}`}
              onMouseEnter={() => setIndex(itemIndex)}
              onClick={() => {
                if (command.id === 'search' && query.trim()) saveRecent(query.trim())
                command.action()
                close()
              }}
            >
              <span className="palette-item__icon">{command.icon}</span>
              <span className="palette-item__label">
                {command.label}
                {command.description && (
                  <span className="palette-item__desc">{command.description}</span>
                )}
              </span>
            </li>
          ))}
          {filtered.length === 0 && (
            <li className="palette-empty" role="status">
              No matches for "{query}".
            </li>
          )}
        </ul>
        <footer className="palette-foot">
          <span>
            <kbd>↑</kbd>
            <kbd>↓</kbd> navigate
          </span>
          <span>
            <kbd>Enter</kbd> open · <kbd>Esc</kbd> close
          </span>
        </footer>
      </section>
    </div>
  )
}
