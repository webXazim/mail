import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import {
  Archive,
  ArrowDownUp,
  CheckSquare,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  Clock3,
  CornerDownLeft,
  CornerDownRight,
  Download,
  FileUp,
  Filter,
  Mail as MailIcon,
  MailCheck,
  MailOpen,
  Palmtree,
  RotateCcw,
  SquarePen,
  Star,
  Tag,
  Trash2,
  X,
} from 'lucide-react'
import { useLocation, useNavigate, useSearchParams } from 'react-router-dom'
import {
  categoryLabels,
  filterMails,
  folderFromPath,
  folderPath,
  folderSlug,
  snoozeAt,
  snoozeOptions,
  sortMails,
  sortOptions,
  type SortKey,
} from '../lib/mail'
import { buildScheduledMail } from '../lib/delivery'
import { exportToMbox, importFromMbox } from '../lib/mbox'
import { MailRow } from '../components/MailRow'
import { draftsApi } from '../services/drafts'
import { foldersApi } from '../services/folders'
import { forwardingApi } from '../services/forwarding'
import { labelsApi } from '../services/labels'
import { receiptsApi } from '../services/receipts'
import { scheduleApi } from '../services/schedule'
import { settingsApi } from '../services/settings'
import { vacationApi } from '../services/vacation'
import { useMail } from '../state/mail/MailContext'
import { useIsMobile } from '../hooks/useIsMobile'
import { useMailListKeyboard } from '../hooks/useMailListKeyboard'
import { composeToDraft, fetchMailPage, formatTime, isRemoteMail, remoteDraftApi } from '../services/remote-mail'
import { NotFoundPage } from './NotFoundPage'
import type { MailActionKind } from '../state/mail/mailboxReducer'
import type { Draft, Mail } from '../types'
import type { RealtimeEvent } from '../services/ws'

const pageSize = 25

const toDraftMail = (draft: Draft): Mail => ({
  id: 'draft-local',
  initials: 'AM',
  sender: 'You',
  email: 'alex@crescentsphere.com',
  subject: draft.subject || '(no subject)',
  preview:
    draft.body.slice(0, 120) ||
    (draft.attachments.length ? 'Draft with attachment' : 'Blank draft'),
  time: 'Just now',
  label: 'Draft',
  color: 'teal',
  unread: false,
  folder: 'Drafts',
})

const moveChoices: string[] = ['Inbox', 'Archive', 'Snoozed', 'Spam', 'Trash']

export function MailListPage() {
  const navigate = useNavigate()
  const { pathname } = useLocation()
  const [searchParams] = useSearchParams()
  const isMobile = useIsMobile()
  const folder = folderFromPath(pathname)
  const query = searchParams.get('q') || ''
  const customMailboxId = pathname.match(/\/mail\/folders\/([^/]+)/)?.[1]
    ? decodeURIComponent(pathname.match(/\/mail\/folders\/([^/]+)/)?.[1] ?? '')
    : null
  const {
    mailbox,
    loading,
    loadError,
    openCompose,
    markRead,
    markUnread,
    markAllRead,
    applyAction,
    moveToFolder,
    emptyTrash,
    toggleLabel,
    toggleStar,
    snooze,
    removeScheduled,
    reload,
    notify,
    importMails,
    mergeRemoteMails,
    undoAction,
    toasts,
    dismissToast,
  } = useMail()
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
  const [remoteRows, setRemoteRows] = useState<Mail[]>([])
  const [remoteTotal, setRemoteTotal] = useState(0)
  const [remoteHasMore, setRemoteHasMore] = useState(false)
  const [remoteAnchor, setRemoteAnchor] = useState<string | null>(null)
  const [remoteQueryState, setRemoteQueryState] = useState<string | null>(null)
  const [remoteLoading, setRemoteLoading] = useState(false)
  const [remoteError, setRemoteError] = useState(false)
  const [mailForwarding, setMailForwarding] = useState(() => forwardingApi.load())
  const [mailVacation, setMailVacation] = useState(() => vacationApi.load())
  const [sheet, setSheet] = useState<Mail | null>(null)
  const [sheetSection, setSheetSection] = useState<'snooze' | 'label' | 'move' | null>(null)

  const draftsFolder = useMemo(() => (folder === 'Drafts' ? draftsApi.load() : null), [folder])
  const [serverDrafts, setServerDrafts] = useState<Mail[]>([])
  const fetchServerDrafts = useCallback(async (): Promise<Mail[]> => {
    if (folder !== 'Drafts' || !isRemoteMail()) return []
    try {
      const summaries = await remoteDraftApi.list()
      return summaries.map((draft) => ({
        id: `draft:${draft.id}`,
        initials: 'You',
        sender: 'You',
        email: '',
        subject: draft.subject || '(no subject)',
        preview: draft.snippet || (draft.has_attachments ? 'Draft with attachment' : 'Blank draft'),
        time: formatTime(draft.updated_at) || 'Just now',
        label: 'Draft',
        color: 'teal',
        unread: false,
        folder: 'Drafts',
      }))
    } catch {
      return []
    }
  }, [folder])
  useEffect(() => {
    let cancelled = false
    void (async () => {
      const rows = await fetchServerDrafts()
      if (!cancelled) setServerDrafts(rows)
    })()
    return () => {
      cancelled = true
    }
  }, [fetchServerDrafts])
  useEffect(() => {
    if (folder !== 'Inbox') return
    let cancelled = false
    void Promise.all([forwardingApi.refresh(), vacationApi.refresh()])
      .then(([forwardResult, vacationResult]) => {
        if (cancelled) return
        setMailForwarding(forwardResult.forwarding)
        setMailVacation(vacationResult.vacation)
      })
      .catch(() => {})
    return () => {
      cancelled = true
    }
  }, [folder])

  const [scheduledList, setScheduledList] = useState(() => scheduleApi.list())
  useEffect(() => {
    if (!isRemoteMail() || folder !== 'Scheduled') return
    let cancelled = false
    const refresh = () => {
      void scheduleApi
        .refresh()
        .then((list) => {
          if (!cancelled) setScheduledList(list)
        })
        .catch(() => {})
    }
    refresh()
    const timer = window.setInterval(refresh, 30000)
    return () => {
      cancelled = true
      window.clearInterval(timer)
    }
  }, [folder])

  useEffect(() => {
    if (!isRemoteMail()) return
    const onRealtime = (incoming: Event) => {
      const detail = (incoming as CustomEvent<RealtimeEvent>).detail
      if (!detail || detail.kind !== 'resource-changed') return
      if (detail.payload.resource === 'drafts' && folder === 'Drafts') {
        void fetchServerDrafts().then(setServerDrafts).catch(() => {})
      }
      if (detail.payload.resource === 'schedule' && folder === 'Scheduled') {
        void scheduleApi.refresh().then(setScheduledList).catch(() => {})
      }
    }
    window.addEventListener('cs-mail-realtime', onRealtime)
    return () => window.removeEventListener('cs-mail-realtime', onRealtime)
  }, [fetchServerDrafts, folder])
  useEffect(() => {
    void receiptsApi.refresh()
  }, [])
  const openServerDraft = useCallback(
    async (mail: Mail) => {
      const id = mail.id.slice('draft:'.length)
      try {
        const compose = await remoteDraftApi.get(id)
        remoteDraftApi.setActiveId(id)
        openCompose(composeToDraft(compose))
      } catch {
        notify('Could not open that draft')
      }
    },
    [openCompose, notify],
  )
  const removeServerDraft = useCallback(
    async (mail: Mail) => {
      const id = mail.id.slice('draft:'.length)
      try {
        await remoteDraftApi.remove(id)
        setServerDrafts(await fetchServerDrafts())
        notify('Draft deleted')
      } catch {
        notify('Could not delete that draft')
      }
    },
    [fetchServerDrafts, notify],
  )
  const labelOptions = useMemo(() => labelsApi.list().map((label) => label.name), [])
  const moveOptions = useMemo(
    () =>
      [...moveChoices, ...foldersApi.list().map((customFolder) => customFolder.name)].filter(
        (item) => item !== folder,
      ),
    [folder],
  )
  const scheduledById = useMemo(
    () => new Map(scheduledList.map((entry) => [entry.id, entry])),
    [scheduledList],
  )
  const retryScheduled = useCallback(
    async (id: string) => {
      try {
        await scheduleApi.retry(id)
        const next = await scheduleApi.refresh()
        setScheduledList(next)
        notify('Scheduled delivery queued for retry')
      } catch (error) {
        notify(error instanceof Error ? error.message : 'Could not retry scheduled delivery')
      }
    },
    [notify],
  )
  const labelOf = useMemo(() => new Map(mailbox.map((mail) => [mail.id, mail.label])), [mailbox])
  const categoryCounts = useMemo(() => {
    const counts: Record<string, number> = {}
    for (const [name, labels] of Object.entries(categoryLabels)) {
      counts[name] = mailbox.filter(
        (mail) => (mail.folder || 'Inbox') === 'Inbox' && labels.includes(mail.label),
      ).length
    }
    return counts
  }, [mailbox])

  const remotePaged = isRemoteMail() && !query && !['Drafts', 'Scheduled', 'Snoozed'].includes(folder)
  const remoteSort = useMemo(() => {
    if (sortKey === 'oldest') return 'received_asc' as const
    if (sortKey === 'sender-az') return 'sender_asc' as const
    if (sortKey === 'sender-za') return 'sender_desc' as const
    if (sortKey === 'subject-az') return 'subject_asc' as const
    if (sortKey === 'subject-za') return 'subject_desc' as const
    return 'received_desc' as const
  }, [sortKey])

  const loadRemotePage = useCallback(async (reset: boolean) => {
    if (!remotePaged) return
    setRemoteLoading(true)
    setRemoteError(false)
    try {
      const result = await fetchMailPage(folder, {
        limit: 50,
        anchor: reset ? null : remoteAnchor,
        queryState: reset ? null : remoteQueryState,
        sort: remoteSort,
        unread: filters.unread,
        starred: filters.starred,
        attachment: filters.attachment,
        mailboxId: customMailboxId,
      })
      if (!reset && result.resetRequired) {
        const fresh = await fetchMailPage(folder, {
          limit: 50,
          sort: remoteSort,
          unread: filters.unread,
          starred: filters.starred,
          attachment: filters.attachment,
          mailboxId: customMailboxId,
        })
        setRemoteRows(fresh.mails)
        mergeRemoteMails(fresh.mails)
        setRemoteTotal(fresh.total)
        setRemoteHasMore(fresh.hasMore)
        setRemoteAnchor(fresh.nextAnchor)
        setRemoteQueryState(fresh.queryState)
        setPage(1)
      } else {
        setRemoteRows((current) => {
          if (reset) return result.mails
          const seen = new Set(current.map((mail) => mail.id))
          return [...current, ...result.mails.filter((mail) => !seen.has(mail.id))]
        })
        mergeRemoteMails(result.mails)
        setRemoteTotal(result.total)
        setRemoteHasMore(result.hasMore)
        setRemoteAnchor(result.nextAnchor)
        setRemoteQueryState(result.queryState)
      }
    } catch {
      setRemoteError(true)
    } finally {
      setRemoteLoading(false)
    }
  }, [remotePaged, folder, customMailboxId, remoteAnchor, remoteQueryState, remoteSort, filters.unread, filters.starred, filters.attachment, mergeRemoteMails])

  useEffect(() => {
    if (!remotePaged) return
    setPage(1)
    setRemoteRows([])
    setRemoteAnchor(null)
    setRemoteQueryState(null)
    void loadRemotePage(true)
    // loadRemotePage intentionally includes cursor state; a reset must only
    // rerun when the view/filter/sort changes, not after every fetched page.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [remotePaged, folder, customMailboxId, remoteSort, filters.unread, filters.starred, filters.attachment])

  const filtered = useMemo(() => {
    if (folder === 'Scheduled') return scheduledList.map(buildScheduledMail)
    if (folder === 'Drafts') {
      if (isRemoteMail()) return serverDrafts
      return draftsFolder ? [toDraftMail(draftsFolder)] : []
    }
    return filterMails(mailbox, folder, query, category)
  }, [folder, mailbox, query, category, draftsFolder, scheduledList, serverDrafts])

  const visiblePool = useMemo(() => {
    if (remotePaged) {
      const current = new Map(mailbox.map((mail) => [mail.id, mail]))
      return remoteRows.map((mail) => current.get(mail.id) ?? mail).filter((mail) => {
        if (filters.unread && !mail.unread) return false
        if (filters.starred && !mail.starred) return false
        if (filters.attachment && !mail.attachment) return false
        if (folder === 'Unread') return mail.unread && mail.folder !== 'Trash'
        if (folder === 'Starred') return Boolean(mail.starred) && mail.folder !== 'Trash'
        if (folder === 'All Mail') return mail.folder !== 'Trash'
        return (mail.folder || 'Inbox') === folder
      })
    }
    let items = filtered
    if (filters.unread) items = items.filter((mail) => mail.unread)
    if (filters.starred) items = items.filter((mail) => mail.starred)
    if (filters.attachment) items = items.filter((mail) => mail.attachment)
    items = sortMails(items, sortKey)
    return items
  }, [remotePaged, remoteRows, mailbox, folder, filtered, sortKey, filters])

  const totalForPaging = remotePaged ? remoteTotal : visiblePool.length
  const pageCount = Math.max(1, Math.ceil(totalForPaging / pageSize))
  const currentPage = Math.min(page, pageCount)
  const visible = useMemo(
    () => visiblePool.slice((currentPage - 1) * pageSize, currentPage * pageSize),
    [visiblePool, currentPage],
  )
  const goToPage = async (next: number) => {
    const target = Math.max(1, Math.min(pageCount, next))
    const required = target * pageSize
    if (remotePaged && required > visiblePool.length && remoteHasMore && !remoteLoading) {
      await loadRemotePage(false)
    }
    setPage(target)
  }
  const activeFilterCount =
    Number(filters.unread) + Number(filters.starred) + Number(filters.attachment)

  const openThread = useCallback(
    (mail: Mail) => {
      if (settingsApi.load().markReadOnOpen) markRead(mail.id)
      if (
        settingsApi.load().sendReadReceipts &&
        mail.email &&
        !mail.email.toLowerCase().endsWith('@crescentsphere.com') &&
        !receiptsApi.has(mail.id)
      ) {
        void receiptsApi.record(mail)
        notify(`Read receipt sent to ${mail.sender}`)
      }
      const base = customMailboxId
        ? `folders/${encodeURIComponent(customMailboxId)}`
        : folderPath(folder)
      navigate(`/mail/${base}/thread/${mail.id}`)
    },
    [folder, customMailboxId, markRead, navigate, notify],
  )
  const openDraft = useCallback(
    (mail?: Mail) => {
      if (mail && isRemoteMail()) {
        void openServerDraft(mail)
        return
      }
      openCompose()
    },
    [openCompose, openServerDraft],
  )
  const openScheduled = useCallback(
    (mail: Mail) => {
      const entry = scheduledById.get(mail.id)
      if (entry) openCompose({ ...entry.draft, scheduledAt: entry.at })
    },
    [scheduledById, openCompose],
  )
  const toggleCheck = useCallback((checkedNext: boolean, id: string) => {
    setChecked((current) =>
      checkedNext ? [...current, id] : current.filter((existing) => existing !== id),
    )
  }, [])
  const toggleAll = (value: boolean) => setChecked(value ? visible.map((mail) => mail.id) : [])
  const runAction = (action: MailActionKind) => {
    applyAction(action, checked)
    setChecked([])
  }
  const quickArchive = useCallback((mail: Mail) => applyAction('archive', [mail.id]), [applyAction])
  const quickTrash = useCallback((mail: Mail) => applyAction('trash', [mail.id]), [applyAction])
  const quickStar = useCallback((mail: Mail) => toggleStar([mail.id]), [toggleStar])

  const mboxInputRef = useRef<HTMLInputElement | null>(null)
  const exportMbox = () => {
    const blob = new Blob([exportToMbox(mailbox)], { type: 'application/mbox' })
    const url = URL.createObjectURL(blob)
    const link = document.createElement('a')
    link.href = url
    link.download = 'cs-mail-export.mbox'
    link.click()
    URL.revokeObjectURL(url)
    notify('Mailbox exported')
  }
  const onImportMbox = (event: React.ChangeEvent<HTMLInputElement>) => {
    const file = event.target.files?.[0]
    if (!file) return
    void file
      .text()
      .then((content) => {
        const existing = new Set(mailbox.map((mail) => mail.id))
        const fresh = importFromMbox(content).filter((mail) => !existing.has(mail.id))
        if (fresh.length) {
          importMails(fresh)
          notify(`Imported ${fresh.length} message${fresh.length === 1 ? '' : 's'}`)
        } else {
          notify('Nothing new to import')
        }
      })
      .finally(() => {
        if (event.target) event.target.value = ''
      })
  }

  const closeMenus = () => {
    setLabelsOpen(false)
    setSnoozeOpen(false)
    setFilterOpen(false)
    setSortOpen(false)
  }
  const readOnly = folder === 'Scheduled' || folder === 'Drafts'
  useEffect(() => {
    const esc = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        closeMenus()
        setSheet(null)
        setSheetSection(null)
      }
    }
    window.addEventListener('click', closeMenus)
    window.addEventListener('keydown', esc)
    return () => {
      window.removeEventListener('click', closeMenus)
      window.removeEventListener('keydown', esc)
    }
  }, [])

  const { cursor, setCursor } = useMailListKeyboard({
    items: visible,
    readOnly,
    onOpen: readOnly
      ? folder === 'Drafts'
        ? openDraft
        : folder === 'Scheduled'
          ? openScheduled
          : () => {}
      : openThread,
    onArchive: (mail) => {
      if (checked.length) runAction('archive')
      else quickArchive(mail)
    },
    onTrash: (mail) => {
      if (checked.length) runAction('trash')
      else quickTrash(mail)
    },
    onStar: (mail) => quickStar(mail),
    onSelect: (mail) => toggleCheck(!checked.includes(mail.id), mail.id),
    onUndo: () => {
      undoAction()
      const toast = toasts.find((item) => item.canUndoAction)
      if (toast) dismissToast(toast.id)
    },
  })

  useEffect(() => {
    const handle = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        setChecked([])
        setCursor(-1)
      }
    }
    window.addEventListener('keydown', handle)
    return () => window.removeEventListener('keydown', handle)
  }, [setCursor])

  const summary =
    visiblePool.length === 0
      ? '0 of 0'
      : `${(currentPage - 1) * pageSize + 1}-${Math.min(currentPage * pageSize, totalForPaging)} of ${totalForPaging}`

  const slug = pathname.match(/\/mail\/([^/]+)/)?.[1]
  const validFolder = Boolean(
    slug && (Object.values(folderSlug).includes(slug) || pathname.startsWith('/mail/folders/')),
  )
  if (!validFolder) return <NotFoundPage />

  return (
    <div className="mail-page">
      <header className="page-head">
        <div>
          <p className="eyebrow">Workspace / Mail</p>
          <h1>{folder}</h1>
          <p>
            {folder === 'Inbox'
              ? 'Everything that needs your attention, in one focused view.'
              : `Messages in ${folder.toLowerCase()}.`}
          </p>
        </div>
        <button className="primary-button" onClick={() => openCompose()}>
          <SquarePen size={16} />
          Compose
        </button>
      </header>
      {folder === 'Inbox' && (mailForwarding.enabled || mailVacation.enabled) && (
        <div className="mail-status">
          {mailForwarding.enabled && mailForwarding.address && (
            <div className="status-banner">
              <CornerDownRight size={14} />
              <span>
                Forwarding is on — incoming mail is sent to <strong>{mailForwarding.address}</strong>
                {mailForwarding.keepCopy ? '' : ' (no copy is kept in this mailbox)'}.
              </span>
            </div>
          )}
          {mailVacation.enabled && (
            <div className="status-banner">
              <Palmtree size={14} />
              <span>
                Auto-reply is on — <strong>&ldquo;{mailVacation.subject}&rdquo;</strong>
                {mailVacation.endsAt
                  ? ` until ${new Date(mailVacation.endsAt).toLocaleDateString([], { month: 'short', day: 'numeric' })}`
                  : ''}
                .
              </span>
            </div>
          )}
        </div>
      )}
      {folder === 'Inbox' && (
        <div className="category-tabs">
          {['Primary', 'Promotions', 'Social', 'Updates'].map((name) => (
            <button
              className={`category-tab ${category === name ? 'category-tab--active' : ''}`}
              aria-pressed={category === name}
              onClick={() => setCategory(name)}
              key={name}
            >
              {name}
              {categoryCounts[name] > 0 && <span>{categoryCounts[name]} new</span>}
            </button>
          ))}
        </div>
      )}
      <div
        className={`mail-toolbar ${checked.length ? 'mail-toolbar--selecting' : 'mail-toolbar--idle'}`}
      >
        <div className="mail-toolbar__bulk">
          {!readOnly && (
            <>
              <input
                type="checkbox"
                aria-label="Select all"
                checked={visible.length > 0 && checked.length === visible.length}
                onChange={(event) => toggleAll(event.target.checked)}
              />
              <button
                className="icon-button"
                aria-label="Archive"
                onClick={() => runAction('archive')}
              >
                <Archive size={17} />
              </button>
              <button
                className="icon-button"
                aria-label="Mark read"
                onClick={() => runAction('read')}
              >
                <MailOpen size={17} />
              </button>
              <button
                className="icon-button"
                aria-label="Mark unread"
                onClick={() => markUnread(checked)}
              >
                <MailIcon size={17} />
              </button>
              <button
                className="icon-button"
                aria-label="Mark all read"
                disabled={visiblePool.length === 0}
                onClick={() => {
                  markAllRead(visiblePool.map((mail) => mail.id))
                  setChecked([])
                }}
              >
                <MailCheck size={17} />
              </button>
              <button
                className="icon-button"
                aria-label="Move to trash"
                onClick={() => runAction('trash')}
              >
                <Trash2 size={17} />
              </button>
              <div className="toolbar-pop">
                <button
                  className="icon-button"
                  aria-label="Add labels"
                  aria-expanded={labelsOpen}
                  onClick={(event) => {
                    event.stopPropagation()
                    setLabelsOpen((value) => !value)
                  }}
                >
                  <Tag size={17} />
                </button>
                {labelsOpen && (
                  <div className="toolbar-menu" role="menu" aria-label="Apply a label">
                    {labelOptions.map((label) => (
                      <button
                        type="button"
                        role="menuitemcheckbox"
                        aria-checked={
                          checked.length > 0 && checked.every((id) => labelOf.get(id) === label)
                        }
                        key={label}
                        onClick={(event) => {
                          event.stopPropagation()
                          toggleLabel(checked, label)
                          setLabelsOpen(false)
                        }}
                      >
                        {label}
                      </button>
                    ))}
                  </div>
                )}
              </div>
              <div className="toolbar-pop">
                <button
                  className="icon-button"
                  aria-label="Snooze"
                  aria-expanded={snoozeOpen}
                  onClick={(event) => {
                    event.stopPropagation()
                    setSnoozeOpen((value) => !value)
                  }}
                >
                  <Clock3 size={17} />
                </button>
                {snoozeOpen && (
                  <div className="toolbar-menu" role="menu" aria-label="Snooze until">
                    {snoozeOptions.map((option) => (
                      <button
                        type="button"
                        role="menuitem"
                        key={option.id}
                        onClick={(event) => {
                          event.stopPropagation()
                          snooze(checked, snoozeAt(option.id))
                          setChecked([])
                          setSnoozeOpen(false)
                        }}
                      >
                        {option.label}
                      </button>
                    ))}
                  </div>
                )}
              </div>
              <select
                aria-label="Move to folder"
                className="move-select"
                defaultValue=""
                onChange={(event) => {
                  const next = event.target.value
                  if (next) {
                    moveToFolder(checked, next)
                    setChecked([])
                    event.target.value = ''
                  }
                }}
              >
                <option value="" disabled>
                  Move to
                </option>
                {moveOptions.map((emailFolder) => (
                  <option key={emailFolder} value={emailFolder}>
                    {emailFolder}
                  </option>
                ))}
              </select>
              {folder === 'Trash' && (
                <div className="toolbar-pop">
                  <button
                    className="icon-button"
                    aria-label="Empty trash"
                    aria-expanded={trashConfirm}
                    onClick={(event) => {
                      event.stopPropagation()
                      if (!trashConfirm) setTrashConfirm(true)
                      else {
                        emptyTrash()
                        setTrashConfirm(false)
                        setChecked([])
                      }
                    }}
                  >
                    {trashConfirm ? <RotateCcw size={17} /> : <Trash2 size={17} />}
                  </button>
                  {trashConfirm && (
                    <div className="toolbar-menu" role="menu">
                      <button
                        type="button"
                        role="menuitem"
                        onClick={(event) => {
                          event.stopPropagation()
                          emptyTrash()
                          setTrashConfirm(false)
                          setChecked([])
                        }}
                      >
                        Permanently delete all trash
                      </button>
                    </div>
                  )}
                </div>
              )}
              {folder === 'Spam' && (
                <button
                  className="icon-button"
                  aria-label="Not spam"
                  onClick={() => {
                    moveToFolder(
                      checked.length
                        ? checked
                        : mailbox.filter((mail) => mail.folder === 'Spam').map((mail) => mail.id),
                      'Inbox',
                    )
                    setChecked([])
                  }}
                >
                  <RotateCcw size={17} />
                </button>
              )}
            </>
          )}
        </div>
        <span className="toolbar-spacer" />
        {folder === 'All Mail' && (
          <>
            <input
              ref={mboxInputRef}
              type="file"
              accept=".mbox"
              hidden
              aria-label="Import mailbox"
              data-testid="mbox-import"
              onChange={onImportMbox}
            />
            <button
              className="icon-button"
              aria-label="Export mailbox"
              title="Download your mailbox as an .mbox file"
              onClick={exportMbox}
            >
              <Download size={17} />
            </button>
            <button
              className="icon-button"
              aria-label="Import mailbox"
              title="Import messages from an .mbox file"
              onClick={() => mboxInputRef.current?.click()}
            >
              <FileUp size={17} />
            </button>
          </>
        )}
        <div className="toolbar-pop">
          <button
            className="toolbar-text-button"
            aria-label="Filter messages"
            aria-expanded={filterOpen}
            onClick={(event) => {
              event.stopPropagation()
              setFilterOpen((value) => !value)
            }}
          >
            <Filter size={14} />
            Filter
            {activeFilterCount > 0 && <span className="toolbar-count">{activeFilterCount}</span>}
          </button>
          {filterOpen && (
            <div
              className="toolbar-menu toolbar-menu--right"
              role="menu"
              aria-label="Filter messages"
            >
              <button
                type="button"
                role="menuitemcheckbox"
                aria-checked={filters.unread}
                onClick={(event) => {
                  event.stopPropagation()
                  setFilters((current) => ({ ...current, unread: !current.unread }))
                }}
              >
                Unread
              </button>
              <button
                type="button"
                role="menuitemcheckbox"
                aria-checked={filters.starred}
                onClick={(event) => {
                  event.stopPropagation()
                  setFilters((current) => ({ ...current, starred: !current.starred }))
                }}
              >
                Starred
              </button>
              <button
                type="button"
                role="menuitemcheckbox"
                aria-checked={filters.attachment}
                onClick={(event) => {
                  event.stopPropagation()
                  setFilters((current) => ({ ...current, attachment: !current.attachment }))
                }}
              >
                Has attachment
              </button>
              {activeFilterCount > 0 && (
                <button
                  type="button"
                  role="menuitem"
                  onClick={(event) => {
                    event.stopPropagation()
                    setFilters({ unread: false, starred: false, attachment: false })
                  }}
                >
                  Clear filters
                </button>
              )}
            </div>
          )}
        </div>
        <div className="toolbar-pop">
          <button
            className="toolbar-text-button"
            aria-label="Sort messages"
            aria-expanded={sortOpen}
            onClick={(event) => {
              event.stopPropagation()
              setSortOpen((value) => !value)
            }}
          >
            <ArrowDownUp size={14} />
            Sort by
          </button>
          {sortOpen && (
            <div
              className="toolbar-menu toolbar-menu--right"
              role="menu"
              aria-label="Sort messages"
            >
              {sortOptions.map((option) => (
                <button
                  type="button"
                  role="menuitemradio"
                  aria-checked={sortKey === option.id}
                  key={option.id}
                  onClick={(event) => {
                    event.stopPropagation()
                    setSortKey(option.id)
                    setSortOpen(false)
                  }}
                >
                  {option.label}
                </button>
              ))}
            </div>
          )}
        </div>
        <small>{summary}</small>
        <button
          className="icon-button"
          aria-label="Previous"
          disabled={currentPage <= 1 || remoteLoading}
          onClick={() => void goToPage(currentPage - 1)}
        >
          <ChevronLeft size={17} />
        </button>
        <button
          className="icon-button"
          aria-label="Next"
          disabled={currentPage >= pageCount || remoteLoading}
          onClick={() => void goToPage(currentPage + 1)}
        >
          <ChevronRight size={17} />
        </button>
      </div>
      <section className="mail-list">
        {loading || (remotePaged && remoteLoading && remoteRows.length === 0) ? (
          <div className="list-state">
            <div className="loading-spinner" />
            <strong>Loading messages</strong>
            <span>Syncing your CS Mail mailbox...</span>
          </div>
        ) : loadError || remoteError ? (
          <div className="list-state" role="alert">
            <strong>We couldn't load your mailbox</strong>
            <span>
              Something went wrong while syncing with the server. Your connection may be offline.
            </span>
            <button className="primary-button" onClick={reload}>
              <RotateCcw size={16} />
              Try again
            </button>
          </div>
        ) : visible.length ? (
          visible.map((mail, index) => (
            <MailRow
              key={mail.id}
              mail={mail}
              checked={readOnly ? false : checked.includes(mail.id)}
              focused={cursor === index}
              onSelect={readOnly ? undefined : toggleCheck}
              onClick={
                readOnly
                  ? folder === 'Drafts'
                    ? (mail) => openDraft(mail)
                    : folder === 'Scheduled'
                      ? openScheduled
                      : () => {}
                  : openThread
              }
              onQuickArchive={readOnly ? undefined : quickArchive}
              onQuickStar={readOnly ? undefined : quickStar}
              onQuickTrash={readOnly ? undefined : quickTrash}
              onQuickRetry={
                readOnly && folder === 'Scheduled' && scheduledById.get(mail.id)?.status === 'dead'
                  ? (mail) => void retryScheduled(mail.id)
                  : undefined
              }
              onQuickCancel={
                readOnly && folder === 'Scheduled'
                  ? (mail) => removeScheduled(mail.id)
                  : readOnly && folder === 'Drafts' && isRemoteMail()
                    ? (mail) => void removeServerDraft(mail)
                    : undefined
              }
              onLongPress={
                readOnly
                  ? undefined
                  : (mail) => {
                      setSheet(mail)
                      setSheetSection(null)
                    }
              }
              swipeable={isMobile && !readOnly}
              onSwipeArchive={isMobile && !readOnly ? quickArchive : undefined}
              onSwipeTrash={isMobile && !readOnly ? quickTrash : undefined}
            />
          ))
        ) : (
          <div className="list-state">
            <strong>
              {query
                ? 'No messages found'
                : folder === 'Scheduled'
                  ? 'Nothing scheduled'
                  : 'Nothing here yet'}
            </strong>
            <span>
              {query
                ? 'Try a different search or remove a filter.'
                : folder === 'Scheduled'
                  ? 'Schedule a message from the composer to see it here.'
                  : `There are no messages in ${folder.toLowerCase()}.`}
            </span>
          </div>
        )}
      </section>
      {sheet && (
        <div className="mobile-sheet" onContextMenu={(event) => event.preventDefault()}>
          <button
            type="button"
            className="mobile-sheet__scrim"
            aria-label="Close actions"
            onClick={() => setSheet(null)}
          />
          <section
            className="mobile-sheet__panel"
            role="dialog"
            aria-modal="true"
            aria-label="Message actions"
          >
            <header className="mobile-sheet__head">
              <div>
                <strong>{sheet.subject}</strong>
                <span>
                  {sheet.sender} · {sheet.email}
                </span>
              </div>
              <button
                type="button"
                className="icon-button"
                aria-label="Close actions"
                onClick={() => {
                  setSheet(null)
                  setSheetSection(null)
                }}
              >
                <X size={17} />
              </button>
            </header>
            {sheet.folder === 'Scheduled' ? (
              <div className="mobile-sheet__list">
                {scheduledById.get(sheet.id)?.status === 'dead' && (
                  <button
                    type="button"
                    onClick={() => {
                      void retryScheduled(sheet.id)
                      setSheet(null)
                    }}
                  >
                    <RotateCcw size={16} />
                    Retry delivery
                  </button>
                )}
                <button
                  type="button"
                  onClick={() => {
                    removeScheduled(sheet.id)
                    setSheet(null)
                  }}
                >
                  <X size={16} />
                  Cancel schedule
                </button>
              </div>
            ) : (
              <>
                <div className="mobile-sheet__grid">
                  <button
                    type="button"
                    onClick={() => {
                      toggleStar([sheet.id])
                      setSheet(null)
                    }}
                  >
                    <Star size={18} fill={sheet.starred ? 'currentColor' : 'none'} />
                    <span>{sheet.starred ? 'Unstar' : 'Star'}</span>
                  </button>
                  <button
                    type="button"
                    onClick={() => {
                      if (sheet.unread) markRead(sheet.id)
                      else markUnread([sheet.id])
                      setSheet(null)
                    }}
                  >
                    {sheet.unread ? <MailOpen size={18} /> : <MailIcon size={18} />}
                    <span>{sheet.unread ? 'Mark read' : 'Mark unread'}</span>
                  </button>
                  <button
                    type="button"
                    onClick={() => {
                      applyAction('archive', [sheet.id])
                      setSheet(null)
                    }}
                  >
                    <Archive size={18} />
                    <span>Archive</span>
                  </button>
                  <button
                    type="button"
                    onClick={() => {
                      applyAction('trash', [sheet.id])
                      setSheet(null)
                    }}
                  >
                    <Trash2 size={18} />
                    <span>Delete</span>
                  </button>
                </div>
                <div className="mobile-sheet__list">
                  {(folder === 'Trash' || folder === 'Spam') && (
                    <button
                      type="button"
                      onClick={() => {
                        moveToFolder([sheet.id], 'Inbox')
                        setSheet(null)
                      }}
                    >
                      <CornerDownLeft size={16} />
                      Restore to inbox
                    </button>
                  )}
                  <button
                    type="button"
                    onClick={() => {
                      toggleCheck(!checked.includes(sheet.id), sheet.id)
                      setSheet(null)
                      setSheetSection(null)
                    }}
                  >
                    <CheckSquare size={16} />
                    {checked.includes(sheet.id) ? 'Deselect' : 'Select message'}
                  </button>
                  <button
                    type="button"
                    className="mobile-sheet__expand"
                    aria-expanded={sheetSection === 'snooze'}
                    onClick={() =>
                      setSheetSection((current) => (current === 'snooze' ? null : 'snooze'))
                    }
                  >
                    <Clock3 size={16} />
                    Snooze
                    <ChevronDown
                      size={14}
                      className={sheetSection === 'snooze' ? 'mobile-sheet__chevron--open' : ''}
                    />
                  </button>
                  {sheetSection === 'snooze' && (
                    <div className="mobile-sheet__sub" role="menu" aria-label="Snooze until">
                      {snoozeOptions.map((option) => (
                        <button
                          type="button"
                          role="menuitem"
                          key={option.id}
                          onClick={() => {
                            snooze([sheet.id], snoozeAt(option.id))
                            setSheet(null)
                          }}
                        >
                          {option.label}
                        </button>
                      ))}
                    </div>
                  )}
                  <button
                    type="button"
                    className="mobile-sheet__expand"
                    aria-expanded={sheetSection === 'label'}
                    onClick={() =>
                      setSheetSection((current) => (current === 'label' ? null : 'label'))
                    }
                  >
                    <Tag size={16} />
                    Apply label
                    <ChevronDown
                      size={14}
                      className={sheetSection === 'label' ? 'mobile-sheet__chevron--open' : ''}
                    />
                  </button>
                  {sheetSection === 'label' && (
                    <div className="mobile-sheet__sub" role="menu" aria-label="Apply a label">
                      {labelOptions.map((label) => (
                        <button
                          type="button"
                          role="menuitem"
                          key={label}
                          onClick={() => {
                            toggleLabel([sheet.id], label)
                            setSheet(null)
                          }}
                        >
                          {label}
                        </button>
                      ))}
                    </div>
                  )}
                  <button
                    type="button"
                    className="mobile-sheet__expand"
                    aria-expanded={sheetSection === 'move'}
                    onClick={() =>
                      setSheetSection((current) => (current === 'move' ? null : 'move'))
                    }
                  >
                    <CornerDownRight size={16} />
                    Move to folder
                    <ChevronDown
                      size={14}
                      className={sheetSection === 'move' ? 'mobile-sheet__chevron--open' : ''}
                    />
                  </button>
                  {sheetSection === 'move' && (
                    <div className="mobile-sheet__sub" role="menu" aria-label="Move to folder">
                      {moveOptions.map((option) => (
                        <button
                          type="button"
                          role="menuitem"
                          key={option}
                          onClick={() => {
                            moveToFolder([sheet.id], option)
                            setSheet(null)
                          }}
                        >
                          {option}
                        </button>
                      ))}
                    </div>
                  )}
                </div>
              </>
            )}
          </section>
        </div>
      )}
    </div>
  )
}
