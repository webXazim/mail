import { useCallback, useEffect, useMemo, useState } from 'react'
import { Archive, ArrowDownUp, ChevronLeft, ChevronRight, Clock3, CornerDownRight, Filter, Mail as MailIcon, MailCheck, MailOpen, Palmtree, RotateCcw, SquarePen, Tag, Trash2 } from 'lucide-react'
import { useLocation, useNavigate, useSearchParams } from 'react-router-dom'
import { categoryLabels, filterMails, folderFromPath, folderPath, snoozeAt, snoozeOptions } from '../lib/mail'
import { MailRow } from '../components/MailRow'
import { draftsApi } from '../services/drafts'
import { foldersApi } from '../services/folders'
import { forwardingApi } from '../services/forwarding'
import { labelsApi } from '../services/labels'
import { scheduleApi } from '../services/schedule'
import { settingsApi } from '../services/settings'
import { vacationApi } from '../services/vacation'
import { useMail } from '../state/mail/MailContext'
import type { MailActionKind } from '../state/mail/mailboxReducer'
import type { Draft, Mail } from '../types'

const pageSize = 25

type SortKey = 'original' | 'newest' | 'oldest' | 'sender-az' | 'sender-za' | 'subject-az' | 'subject-za'

const sortOptions: { id: SortKey; label: string }[] = [
  { id: 'newest', label: 'Newest first' },
  { id: 'oldest', label: 'Oldest first' },
  { id: 'sender-az', label: 'Sender (A to Z)' },
  { id: 'sender-za', label: 'Sender (Z to A)' },
  { id: 'subject-az', label: 'Subject (A to Z)' },
  { id: 'subject-za', label: 'Subject (Z to A)' },
]

const timeRank = (time: string): number => {
  const t = time.toLowerCase().trim()
  if (t.includes('just now')) return Date.now()
  const ago = t.match(/(\d+)\s*(m|h)\s*ago/)
  if (ago) {
    const n = Number(ago[1])
    return Date.now() - n * (ago[2] === 'h' ? 3600 : 60) * 1000
  }
  const mer = t.match(/^(\d{1,2}):(\d{2})\s*(am|pm)$/)
  if (mer) {
    let h = Number(mer[1]) % 12
    if (mer[3] === 'pm') h += 12
    const base = new Date()
    base.setHours(h, Number(mer[2]), 0, 0)
    return base.getTime()
  }
  if (t === 'yesterday') {
    const y = new Date()
    return new Date(y.getFullYear(), y.getMonth(), y.getDate() - 1).getTime()
  }
  const md = t.match(/^(\w{3})\s+(\d{1,2})$/)
  if (md) {
    const month = ['jan', 'feb', 'mar', 'apr', 'may', 'jun', 'jul', 'aug', 'sep', 'oct', 'nov', 'dec'].indexOf(md[1])
    if (month >= 0) {
      const y = new Date()
      return new Date(y.getFullYear(), month, Number(md[2])).getTime()
    }
  }
  const wd = ['sun', 'mon', 'tue', 'wed', 'thu', 'fri', 'sat'].indexOf(t.slice(0, 3))
  if (wd >= 0) {
    const base = new Date()
    return new Date(base.getFullYear(), base.getMonth(), base.getDate() - ((base.getDay() - wd + 7) % 7)).getTime()
  }
  return 0
}

const toScheduledMail = (entry: { id: string; draft: { to: string; subject: string; body: string; attachments: string[] }; at: string }): Mail => ({
  id: entry.id,
  initials: 'AM',
  sender: 'You',
  email: entry.draft.to || 'Add recipients to send',
  subject: entry.draft.subject || '(no subject)',
  preview: entry.draft.body.slice(0, 120) || (entry.draft.attachments.length ? 'Scheduled draft with attachment' : 'Blank scheduled draft'),
  time: new Date(entry.at).toLocaleString([], { month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit' }),
  label: 'Scheduled',
  color: 'teal',
  unread: false,
  folder: 'Scheduled',
})

const toDraftMail = (draft: Draft): Mail => ({
  id: 'draft-local',
  initials: 'AM',
  sender: 'You',
  email: 'alex@harbor.co',
  subject: draft.subject || '(no subject)',
  preview: draft.body.slice(0, 120) || (draft.attachments.length ? 'Draft with attachment' : 'Blank draft'),
  time: 'Just now',
  label: 'Draft',
  color: 'teal',
  unread: false,
  folder: 'Drafts',
})

const moveChoices: string[] = ['Inbox', 'Archive', 'Snoozed', 'Spam', 'Trash']

const isEditableTarget = (target: EventTarget | null) =>
  target instanceof HTMLElement && Boolean(target.closest('input, textarea, select, [contenteditable="true"]'))

export function MailListPage() {
  const navigate = useNavigate()
  const { pathname } = useLocation()
  const [searchParams] = useSearchParams()
  const folder = folderFromPath(pathname)
  const query = searchParams.get('q') || ''
  const { mailbox, loading, loadError, scheduledCount, openCompose, markRead, markUnread, markAllRead, applyAction, moveToFolder, emptyTrash, toggleLabel, toggleStar, snooze, removeScheduled, reload } = useMail()
  const [category, setCategory] = useState('Primary')
  const [checked, setChecked] = useState<string[]>([])
  const [labelsOpen, setLabelsOpen] = useState(false)
  const [snoozeOpen, setSnoozeOpen] = useState(false)
  const [filterOpen, setFilterOpen] = useState(false)
  const [sortOpen, setSortOpen] = useState(false)
  const [sortKey, setSortKey] = useState<SortKey>('original')
  const [filters, setFilters] = useState({ unread: false, starred: false, attachment: false })
  const [trashConfirm, setTrashConfirm] = useState(false)
  const [page, setPage] = useState(1)

  const draftsFolder = useMemo(() => (folder === 'Drafts' ? draftsApi.load() : null), [folder])
  const labelOptions = useMemo(() => labelsApi.list().map(label => label.name), [])
  const moveOptions = useMemo(() => [...moveChoices, ...foldersApi.list().map(customFolder => customFolder.name)].filter(item => item !== folder), [folder])
  const scheduledById = useMemo(() => new Map(scheduleApi.list().map(entry => [entry.id, entry])), [])
  const labelOf = useMemo(() => new Map(mailbox.map(mail => [mail.id, mail.label])), [mailbox])
  const categoryCounts = useMemo(() => {
    const counts: Record<string, number> = {}
    for (const [name, labels] of Object.entries(categoryLabels)) {
      counts[name] = mailbox.filter(mail => (mail.folder || 'Inbox') === 'Inbox' && labels.includes(mail.label)).length
    }
    return counts
  }, [mailbox])

  const filtered = useMemo(() => {
    if (folder === 'Scheduled') return scheduleApi.list().slice(0, scheduledCount).map(toScheduledMail)
    if (folder === 'Drafts') return draftsFolder ? [toDraftMail(draftsFolder)] : []
    return filterMails(mailbox, folder, query, category)
  }, [folder, mailbox, query, category, draftsFolder, scheduledCount])

  const visiblePool = useMemo(() => {
    let items = filtered
    if (filters.unread) items = items.filter(mail => mail.unread)
    if (filters.starred) items = items.filter(mail => mail.starred)
    if (filters.attachment) items = items.filter(mail => mail.attachment)
    if (sortKey !== 'original') {
      const copy = [...items]
      if (sortKey === 'newest') copy.sort((a, b) => timeRank(b.time) - timeRank(a.time))
      else if (sortKey === 'oldest') copy.sort((a, b) => timeRank(a.time) - timeRank(b.time))
      else if (sortKey === 'sender-az') copy.sort((a, b) => a.sender.localeCompare(b.sender))
      else if (sortKey === 'sender-za') copy.sort((a, b) => b.sender.localeCompare(a.sender))
      else if (sortKey === 'subject-az') copy.sort((a, b) => a.subject.localeCompare(b.subject))
      else copy.sort((a, b) => b.subject.localeCompare(a.subject))
      items = copy
    }
    return items
  }, [filtered, sortKey, filters])

  const pageCount = Math.max(1, Math.ceil(visiblePool.length / pageSize))
  const currentPage = Math.min(page, pageCount)
  const visible = useMemo(() => visiblePool.slice((currentPage - 1) * pageSize, currentPage * pageSize), [visiblePool, currentPage])
  const goToPage = (next: number) => setPage(Math.max(1, Math.min(pageCount, next)))
  const activeFilterCount = Number(filters.unread) + Number(filters.starred) + Number(filters.attachment)

  const openThread = useCallback((mail: Mail) => { if (settingsApi.load().markReadOnOpen) markRead(mail.id); navigate(`/mail/${folderPath(folder)}/thread/${mail.id}`) }, [folder, markRead, navigate])
  const openDraft = useCallback(() => { openCompose() }, [openCompose])
  const openScheduled = useCallback((mail: Mail) => {
    const entry = scheduledById.get(mail.id)
    if (entry) openCompose({ ...entry.draft, scheduledAt: entry.at })
  }, [scheduledById, openCompose])
  const toggleCheck = useCallback((checkedNext: boolean, id: string) => { setChecked(current => checkedNext ? [...current, id] : current.filter(existing => existing !== id)) }, [])
  const toggleAll = (value: boolean) => setChecked(value ? visible.map(mail => mail.id) : [])
  const runAction = (action: MailActionKind) => { applyAction(action, checked); setChecked([]) }
  const quickArchive = useCallback((mail: Mail) => applyAction('archive', [mail.id]), [applyAction])
  const quickTrash = useCallback((mail: Mail) => applyAction('trash', [mail.id]), [applyAction])
  const quickStar = useCallback((mail: Mail) => toggleStar([mail.id]), [toggleStar])

  const closeMenus = () => { setLabelsOpen(false); setSnoozeOpen(false); setFilterOpen(false); setSortOpen(false) }
  useEffect(() => {
    const esc = (event: KeyboardEvent) => { if (event.key === 'Escape') closeMenus() }
    window.addEventListener('click', closeMenus)
    window.addEventListener('keydown', esc)
    return () => {
      window.removeEventListener('click', closeMenus)
      window.removeEventListener('keydown', esc)
    }
  }, [])

  useEffect(() => {
    const handle = (event: KeyboardEvent) => {
      if (isEditableTarget(event.target)) return
      if (checked.length && event.key.toLowerCase() === 'e') { applyAction('archive', checked); setChecked([]) }
      if (checked.length && event.key === '#') { applyAction('trash', checked); setChecked([]) }
      if (event.key === 'Escape') setChecked([])
    }
    window.addEventListener('keydown', handle)
    return () => window.removeEventListener('keydown', handle)
  }, [checked, applyAction])

  const summary = visiblePool.length === 0 ? '0 of 0' : `${(currentPage - 1) * pageSize + 1}-${Math.min(currentPage * pageSize, visiblePool.length)} of ${visiblePool.length}`
  const readOnly = folder === 'Scheduled' || folder === 'Drafts'

  return (
    <div className="mail-page">
      <header className="page-head">
        <div>
          <p className="eyebrow">Workspace / Mail</p>
          <h1>{folder}</h1>
          <p>{folder === 'Inbox' ? 'Everything that needs your attention, in one focused view.' : `Messages in ${folder.toLowerCase()}.`}</p>
        </div>
        <button className="primary-button" onClick={() => openCompose()}><SquarePen size={16} />Compose</button>
      </header>
      {folder === 'Inbox' && (() => {
        const forwarding = forwardingApi.load()
        const vacation = vacationApi.load()
        return (forwarding.enabled || vacation.enabled) ? (
          <div className="mail-status">
            {forwarding.enabled && forwarding.address && (
              <div className="status-banner"><CornerDownRight size={14} /><span>Forwarding is on — mail to <strong>alex@harbor.co</strong> is sent to <strong>{forwarding.address}</strong>{forwarding.keepCopy ? '' : ' (no copy is kept in this mailbox)'}.</span></div>
            )}
            {vacation.enabled && (
              <div className="status-banner"><Palmtree size={14} /><span>Auto-reply is on — <strong>&ldquo;{vacation.subject}&rdquo;</strong>{vacation.endsAt ? ` until ${new Date(vacation.endsAt).toLocaleDateString([], { month: 'short', day: 'numeric' })}` : ''}.</span></div>
            )}
          </div>
        ) : null
      })()}
      {folder === 'Inbox' && (
        <div className="category-tabs">
          {['Primary', 'Promotions', 'Social', 'Updates'].map(name => (
            <button className={`category-tab ${category === name ? 'category-tab--active' : ''}`} aria-pressed={category === name} onClick={() => setCategory(name)} key={name}>
              {name}{categoryCounts[name] > 0 && <span>{categoryCounts[name]} new</span>}
            </button>
          ))}
        </div>
      )}
      <div className="mail-toolbar">
        {!readOnly && (
          <>
            <input type="checkbox" aria-label="Select all" checked={visible.length > 0 && checked.length === visible.length} onChange={event => toggleAll(event.target.checked)} />
            <button className="icon-button" aria-label="Archive" onClick={() => runAction('archive')}><Archive size={17} /></button>
            <button className="icon-button" aria-label="Mark read" onClick={() => runAction('read')}><MailOpen size={17} /></button>
            <button className="icon-button" aria-label="Mark unread" onClick={() => markUnread(checked)}><MailIcon size={17} /></button>
            <button className="icon-button" aria-label="Mark all read" disabled={visiblePool.length === 0} onClick={() => { markAllRead(visiblePool.map(mail => mail.id)); setChecked([]) }}><MailCheck size={17} /></button>
            <button className="icon-button" aria-label="Move to trash" onClick={() => runAction('trash')}><Trash2 size={17} /></button>
            <div className="toolbar-pop">
              <button className="icon-button" aria-label="Add labels" aria-expanded={labelsOpen} onClick={event => { event.stopPropagation(); setLabelsOpen(value => !value) }}><Tag size={17} /></button>
              {labelsOpen && (
                <div className="toolbar-menu" role="menu" aria-label="Apply a label">
                  {labelOptions.map(label => (
                    <button type="button" role="menuitemcheckbox" aria-checked={checked.length > 0 && checked.every(id => labelOf.get(id) === label)} key={label} onClick={event => { event.stopPropagation(); toggleLabel(checked, label); setLabelsOpen(false) }}>{label}</button>
                  ))}
                </div>
              )}
            </div>
            <div className="toolbar-pop">
              <button className="icon-button" aria-label="Snooze" aria-expanded={snoozeOpen} onClick={event => { event.stopPropagation(); setSnoozeOpen(value => !value) }}><Clock3 size={17} /></button>
              {snoozeOpen && (
                <div className="toolbar-menu" role="menu" aria-label="Snooze until">
                  {snoozeOptions.map(option => (
                    <button type="button" role="menuitem" key={option.id} onClick={event => { event.stopPropagation(); snooze(checked, snoozeAt(option.id)); setChecked([]); setSnoozeOpen(false) }}>{option.label}</button>
                  ))}
                </div>
              )}
            </div>
            <select aria-label="Move to folder" className="move-select" defaultValue="" onChange={event => { const next = event.target.value; if (next) { moveToFolder(checked, next); setChecked([]); event.target.value = '' } }}>
              <option value="" disabled>Move to</option>
              {moveOptions.map(emailFolder => <option key={emailFolder} value={emailFolder}>{emailFolder}</option>)}
            </select>
            {folder === 'Trash' && (
              <div className="toolbar-pop">
                <button className="icon-button" aria-label="Empty trash" aria-expanded={trashConfirm} onClick={event => { event.stopPropagation(); if (!trashConfirm) setTrashConfirm(true); else { emptyTrash(); setTrashConfirm(false); setChecked([]) } }}>
                  {trashConfirm ? <RotateCcw size={17} /> : <Trash2 size={17} />}
                </button>
                {trashConfirm && (
                  <div className="toolbar-menu" role="menu">
                    <button type="button" role="menuitem" onClick={event => { event.stopPropagation(); emptyTrash(); setTrashConfirm(false); setChecked([]) }}>Permanently delete all trash</button>
                  </div>
                )}
              </div>
            )}
            {folder === 'Spam' && (
              <button className="icon-button" aria-label="Not spam" onClick={() => { moveToFolder(checked.length ? checked : mailbox.filter(mail => mail.folder === 'Spam').map(mail => mail.id), 'Inbox'); setChecked([]) }}><RotateCcw size={17} /></button>
            )}
          </>
        )}
        <span className="toolbar-spacer" />
        <div className="toolbar-pop">
          <button className="toolbar-text-button" aria-label="Filter messages" aria-expanded={filterOpen} onClick={event => { event.stopPropagation(); setFilterOpen(value => !value) }}>
            <Filter size={14} />Filter{activeFilterCount > 0 && <span className="toolbar-count">{activeFilterCount}</span>}
          </button>
          {filterOpen && (
            <div className="toolbar-menu toolbar-menu--right" role="menu" aria-label="Filter messages">
              <button type="button" role="menuitemcheckbox" aria-checked={filters.unread} onClick={event => { event.stopPropagation(); setFilters(current => ({ ...current, unread: !current.unread })) }}>Unread</button>
              <button type="button" role="menuitemcheckbox" aria-checked={filters.starred} onClick={event => { event.stopPropagation(); setFilters(current => ({ ...current, starred: !current.starred })) }}>Starred</button>
              <button type="button" role="menuitemcheckbox" aria-checked={filters.attachment} onClick={event => { event.stopPropagation(); setFilters(current => ({ ...current, attachment: !current.attachment })) }}>Has attachment</button>
              {activeFilterCount > 0 && <button type="button" role="menuitem" onClick={event => { event.stopPropagation(); setFilters({ unread: false, starred: false, attachment: false }) }}>Clear filters</button>}
            </div>
          )}
        </div>
        <div className="toolbar-pop">
          <button className="toolbar-text-button" aria-label="Sort messages" aria-expanded={sortOpen} onClick={event => { event.stopPropagation(); setSortOpen(value => !value) }}>
            <ArrowDownUp size={14} />Sort by
          </button>
          {sortOpen && (
            <div className="toolbar-menu toolbar-menu--right" role="menu" aria-label="Sort messages">
              {sortOptions.map(option => (
                <button type="button" role="menuitemradio" aria-checked={sortKey === option.id} key={option.id} onClick={event => { event.stopPropagation(); setSortKey(option.id); setSortOpen(false) }}>{option.label}</button>
              ))}
            </div>
          )}
        </div>
        <small>{summary}</small>
        <button className="icon-button" aria-label="Previous" disabled={currentPage <= 1} onClick={() => goToPage(currentPage - 1)}><ChevronLeft size={17} /></button>
        <button className="icon-button" aria-label="Next" disabled={currentPage >= pageCount} onClick={() => goToPage(currentPage + 1)}><ChevronRight size={17} /></button>
      </div>
      <section className="mail-list">
        {loading ? (
          <div className="list-state"><div className="loading-spinner" /><strong>Loading messages</strong><span>Syncing your Harbor Mailbox...</span></div>
        ) : loadError ? (
          <div className="list-state" role="alert">
            <strong>We couldn't load your mailbox</strong>
            <span>Something went wrong while syncing with the server. Your connection may be offline.</span>
            <button className="primary-button" onClick={reload}><RotateCcw size={16} />Try again</button>
          </div>
        ) : visible.length ? (
          visible.map(mail => (
            <MailRow
              key={mail.id}
              mail={mail}
              checked={readOnly ? false : checked.includes(mail.id)}
              onSelect={readOnly ? undefined : toggleCheck}
              onClick={readOnly ? (folder === 'Drafts' ? openDraft : folder === 'Scheduled' ? openScheduled : () => {}) : openThread}
              onQuickArchive={readOnly ? undefined : quickArchive}
              onQuickStar={readOnly ? undefined : quickStar}
              onQuickTrash={readOnly ? undefined : quickTrash}
              onQuickCancel={readOnly && folder === 'Scheduled' ? mail => removeScheduled(mail.id) : undefined}
            />
          ))
        ) : (
          <div className="list-state"><strong>{query ? 'No messages found' : folder === 'Scheduled' ? 'Nothing scheduled' : 'Nothing here yet'}</strong><span>{query ? 'Try a different search or remove a filter.' : folder === 'Scheduled' ? 'Schedule a message from the composer to see it here.' : `There are no messages in ${folder.toLowerCase()}.`}</span></div>
        )}
      </section>
    </div>
  )
}