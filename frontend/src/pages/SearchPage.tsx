import { useCallback, useEffect, useMemo, useState } from 'react'
import {
  Archive,
  ArrowDownUp,
  Check,
  CheckSquare,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  Clock3,
  CornerDownLeft,
  CornerDownRight,
  Filter,
  Mail as MailIcon,
  MailOpen,
  SquarePen,
  Star,
  Tag,
  Trash2,
  X,
} from 'lucide-react'
import { useLocation, useNavigate, useSearchParams } from 'react-router-dom'
import {
  filterMails,
  folderPath,
  snoozeAt,
  snoozeOptions,
  sortMails,
  sortOptions,
  type SortKey,
} from '../lib/mail'
import { foldersApi } from '../services/folders'
import { labelsApi } from '../services/labels'
import { MailRow } from '../components/MailRow'
import { AdvancedSearch } from '../components/AdvancedSearch'
import { useIsMobile } from '../hooks/useIsMobile'
import { useMail } from '../state/mail/MailContext'
import { useMailListKeyboard } from '../hooks/useMailListKeyboard'
import { isRemoteMail, searchMail } from '../services/remote-mail'
import type { Mail } from '../types'

const PAGE_SIZE = 25

export function SearchPage() {
  const navigate = useNavigate()
  const { pathname } = useLocation()
  const [searchParams] = useSearchParams()
  const query = searchParams.get('q') || ''
  const {
    mailbox,
    loading,
    loadError,
    openCompose,
    markRead,
    markUnread,
    applyAction,
    toggleLabel,
    toggleStar,
    snooze,
    moveToFolder,
    reload,
    undoAction,
    toasts,
    dismissToast,
  } = useMail()

  const [sortKey, setSortKey] = useState<SortKey>('original')
  const [sortOpen, setSortOpen] = useState(false)
  const [page, setPage] = useState(1)
  const [checked, setChecked] = useState<string[]>([])
  const [sheet, setSheet] = useState<Mail | null>(null)
  const [sheetSection, setSheetSection] = useState<'snooze' | 'label' | 'move' | null>(null)
  const [advancedOpen, setAdvancedOpen] = useState(false)
  const isMobile = useIsMobile()

  const labelOptions = useMemo(() => labelsApi.list().map((label) => label.name), [])
  const moveOptions = useMemo(
    () => [
      'Inbox',
      'Archive',
      'Snoozed',
      'Spam',
      'Trash',
      ...foldersApi.list().map((customFolder) => customFolder.name),
    ],
    [],
  )

  const remoteMode = isRemoteMail()
  const [remoteResults, setRemoteResults] = useState<Mail[]>([])
  const [remoteTotal, setRemoteTotal] = useState(0)
  const [remoteHasMore, setRemoteHasMore] = useState(false)
  const [remoteAnchor, setRemoteAnchor] = useState<string | null>(null)
  const [remoteQueryState, setRemoteQueryState] = useState<string | null>(null)
  const [remoteLoading, setRemoteLoading] = useState(false)
  const [remoteError, setRemoteError] = useState<string | null>(null)

  const remoteSort = useMemo(() => {
    if (sortKey === 'oldest') return 'received_asc' as const
    if (sortKey === 'sender-az') return 'sender_asc' as const
    if (sortKey === 'sender-za') return 'sender_desc' as const
    if (sortKey === 'subject-az') return 'subject_asc' as const
    if (sortKey === 'subject-za') return 'subject_desc' as const
    return 'received_desc' as const
  }, [sortKey])

  const loadRemoteSearch = useCallback(
    async (reset: boolean): Promise<boolean> => {
      if (!remoteMode || !query.trim()) return false
      setRemoteLoading(true)
      setRemoteError(null)
      try {
        const result = await searchMail(query, {
          limit: 50,
          anchor: reset ? null : remoteAnchor,
          queryState: reset ? null : remoteQueryState,
          sort: remoteSort,
        })
        if (!reset && result.resetRequired) {
          const fresh = await searchMail(query, { limit: 50, sort: remoteSort })
          setRemoteResults(fresh.mails)
          setRemoteTotal(fresh.total)
          setRemoteHasMore(fresh.hasMore)
          setRemoteAnchor(fresh.nextAnchor)
          setRemoteQueryState(fresh.queryState)
          setPage(1)
          return true
        }
        setRemoteResults((current) => {
          if (reset) return result.mails
          const seen = new Set(current.map((mail) => mail.id))
          return [...current, ...result.mails.filter((mail) => !seen.has(mail.id))]
        })
        setRemoteTotal(result.total)
        setRemoteHasMore(result.hasMore)
        setRemoteAnchor(result.nextAnchor)
        setRemoteQueryState(result.queryState)
        return false
      } catch (error) {
        setRemoteError(error instanceof Error ? error.message : 'Search failed')
        return false
      } finally {
        setRemoteLoading(false)
      }
    },
    [query, remoteMode, remoteAnchor, remoteQueryState, remoteSort],
  )

  useEffect(() => {
    setPage(1)
    setChecked([])
    setRemoteResults([])
    setRemoteTotal(0)
    setRemoteHasMore(false)
    setRemoteAnchor(null)
    setRemoteQueryState(null)
    setRemoteError(null)
    if (remoteMode && query.trim()) void loadRemoteSearch(true)
    // Cursor state is intentionally excluded: changing it means another page
    // arrived, not that the search definition changed.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [query, remoteMode, remoteSort])

  const handleSort = useCallback(
    (nextKey: SortKey) => {
      setSortKey(nextKey)
      setSortOpen(false)
    },
    [setSortKey, setSortOpen],
  )

  const list = useMemo(() => {
    if (!query) return []
    if (remoteMode) return remoteResults
    return filterMails(mailbox, 'All Mail', query)
  }, [mailbox, query, remoteMode, remoteResults])

  // Remote results are already globally ordered by JMAP. Re-sorting only the
  // currently fetched window would corrupt ordering across later pages.
  const sorted = useMemo(
    () => (remoteMode ? list : sortMails(list, sortKey)),
    [list, remoteMode, sortKey],
  )
  const totalResults = remoteMode ? remoteTotal : sorted.length
  const pageCount = Math.max(1, Math.ceil(totalResults / PAGE_SIZE))
  const currentPage = Math.min(page, pageCount)
  const visible = useMemo(
    () => sorted.slice((currentPage - 1) * PAGE_SIZE, currentPage * PAGE_SIZE),
    [sorted, currentPage],
  )

  const goToPage = useCallback(
    async (next: number) => {
      const target = Math.max(1, Math.min(pageCount, next))
      const required = target * PAGE_SIZE
      if (
        remoteMode &&
        required > sorted.length &&
        remoteHasMore &&
        !remoteLoading
      ) {
        const reset = await loadRemoteSearch(false)
        if (reset) return
      }
      setPage(target)
    },
    [loadRemoteSearch, pageCount, remoteHasMore, remoteLoading, remoteMode, sorted.length],
  )

  const summary = query
    ? `${totalResults} result${totalResults === 1 ? '' : 's'} for “${query}”`
    : totalResults
      ? `${totalResults} message${totalResults === 1 ? '' : 's'}`
      : 'No messages'

  const openThread = useCallback(
    (mail: Mail) =>
      navigate(`/mail/${folderPath(mail.folder || 'Inbox')}/thread/${mail.id}`, {
        state: { background: pathname, mail },
      }),
    [navigate, pathname],
  )

  const toggleCheck = useCallback(
    (isChecked: boolean, id: string) =>
      setChecked((prev) =>
        isChecked ? (prev.includes(id) ? prev : [...prev, id]) : prev.filter((x) => x !== id),
      ),
    [setChecked],
  )

  const toggleAll = useCallback(
    (isChecked: boolean) => setChecked(isChecked ? visible.map((m) => m.id) : []),
    [setChecked, visible],
  )
  const clearSelection = useCallback(() => setChecked([]), [setChecked])

  const quickArchive = useCallback((mail: Mail) => applyAction('archive', [mail.id]), [applyAction])
  const quickStar = useCallback((mail: Mail) => toggleStar([mail.id]), [toggleStar])
  const quickTrash = useCallback((mail: Mail) => applyAction('trash', [mail.id]), [applyAction])

  const { cursor } = useMailListKeyboard({
    items: visible,
    onOpen: openThread,
    onArchive: (mail) => {
      if (checked.length) {
        applyAction('archive', checked)
        clearSelection()
      } else quickArchive(mail)
    },
    onTrash: (mail) => {
      if (checked.length) {
        applyAction('trash', checked)
        clearSelection()
      } else quickTrash(mail)
    },
    onStar: (mail) => quickStar(mail),
    onSelect: (mail) => toggleCheck(!checked.includes(mail.id), mail.id),
    onUndo: () => {
      undoAction()
      const toast = toasts.find((item) => item.canUndoAction)
      if (toast) dismissToast(toast.id)
    },
  })

  const submitAdvanced = useCallback(
    (q: string) => {
      setAdvancedOpen(false)
      navigate(q ? `/mail/search?q=${encodeURIComponent(q)}` : '/mail/search')
    },
    [navigate, setAdvancedOpen],
  )

  const closeAdvanced = useCallback(() => setAdvancedOpen(false), [setAdvancedOpen])

  if (loading || loadError) {
    return (
      <div className="mail-page">
        <div className="page-head">
          <div>
            <p className="eyebrow">Workspace / Search</p>
            <h1>Search mail</h1>
          </div>
        </div>
        {loadError ? (
          <div className="list-state" role="alert">
            <strong>We couldn't load your mail</strong>
            <span>
              Something went wrong while syncing with the server. Your connection may be offline.
            </span>
            <button className="primary-button" onClick={reload}>
              <X size={16} />
              Try again
            </button>
          </div>
        ) : (
          <div className="list-state">
            <div className="loading-spinner" />
            <strong>Loading messages</strong>
            <span>Searching your CS Mail mailbox...</span>
          </div>
        )}
      </div>
    )
  }

  return (
    <div className="mail-page">
      <header className="page-head page-head--search">
        <div>
          <p className="eyebrow">Workspace / Search</p>
          <h1>Search mail</h1>
          <p>
            {query ? (
              <>
                Across all of your mailboxes for <strong>{query}</strong>.
              </>
            ) : (
              'Search across all your mailboxes with Gmail-style operators.'
            )}
          </p>
        </div>
        <button className="primary-button" onClick={() => openCompose()}>
          <SquarePen size={16} />
          Compose
        </button>
      </header>

      <div
        className={`mail-toolbar ${checked.length ? 'mail-toolbar--selecting' : 'mail-toolbar--idle'}`}
      >
        <div className="mail-toolbar__bulk">
          <input
            type="checkbox"
            aria-label="Select all"
            checked={visible.length > 0 && checked.length === visible.length}
            onChange={(event) => toggleAll(event.target.checked)}
          />
          {checked.length > 0 && (
            <>
              <button
                className="icon-button"
                aria-label="Archive"
                onClick={() => {
                  applyAction('archive', checked)
                  clearSelection()
                }}
              >
                <Archive size={17} />
              </button>
              <button
                className="icon-button"
                aria-label="Mark read"
                onClick={() => {
                  applyAction('read', checked)
                  clearSelection()
                }}
              >
                <MailOpen size={17} />
              </button>
              <button
                className="icon-button"
                aria-label="Mark unread"
                onClick={() => {
                  markUnread(checked)
                  clearSelection()
                }}
              >
                <MailIcon size={17} />
              </button>
              <button
                className="icon-button"
                aria-label="Move to trash"
                onClick={() => {
                  applyAction('trash', checked)
                  clearSelection()
                }}
              >
                <Trash2 size={17} />
              </button>
              <select
                aria-label="Move to folder"
                className="move-select"
                defaultValue=""
                onChange={(event) => {
                  const next = event.target.value
                  if (next) {
                    moveToFolder(checked, next)
                    clearSelection()
                    event.target.value = ''
                  }
                }}
              >
                <option value="" disabled>
                  Move to
                </option>
                {moveOptions.map((mailFolder) => (
                  <option key={mailFolder} value={mailFolder}>
                    {mailFolder}
                  </option>
                ))}
              </select>
            </>
          )}
        </div>
        <span className="toolbar-spacer" />
        {!isMobile && query && (
          <button
            type="button"
            className="toolbar-text-button"
            aria-label="Clear search"
            onClick={() => navigate('/mail/search')}
          >
            <X size={14} />
            Clear
          </button>
        )}
        <div className="toolbar-pop">
          <button
            type="button"
            className="toolbar-text-button"
            aria-label="Search options"
            aria-expanded={advancedOpen}
            onClick={(event) => {
              event.stopPropagation()
              setAdvancedOpen((value) => !value)
            }}
          >
            <Filter size={14} />
            Search options
          </button>
          {!isMobile && advancedOpen && (
            <div
              className="toolbar-menu toolbar-menu--right"
              onClick={(event) => event.stopPropagation()}
            >
              <AdvancedSearch onSubmit={submitAdvanced} onClose={closeAdvanced} />
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
          {!isMobile && sortOpen && (
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
                    handleSort(option.id)
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
          disabled={currentPage <= 1}
          onClick={() => void goToPage(currentPage - 1)}
        >
          <ChevronLeft size={17} />
        </button>
        <button
          className="icon-button"
          aria-label="Next"
          disabled={currentPage >= pageCount}
          onClick={() => void goToPage(currentPage + 1)}
        >
          <ChevronRight size={17} />
        </button>
      </div>

      <section className="mail-list">
        {remoteMode && remoteError ? (
          <div className="list-state" role="alert">
            <strong>Search could not be completed</strong>
            <span>{remoteError}</span>
            <button className="primary-button" onClick={() => void loadRemoteSearch(true)}>
              Try again
            </button>
          </div>
        ) : remoteMode && remoteLoading && !visible.length ? (
          <div className="list-state">
            <div className="loading-spinner" />
            <strong>Searching mail</strong>
            <span>Searching your server mailbox…</span>
          </div>
        ) : visible.length ? (
          visible.map((mail, index) => (
            <MailRow
              key={mail.id}
              mail={mail}
              checked={checked.includes(mail.id)}
              focused={cursor === index}
              onSelect={toggleCheck}
              onClick={openThread}
              onQuickArchive={quickArchive}
              onQuickStar={quickStar}
              onQuickTrash={quickTrash}
              onLongPress={(mail) => {
                setSheet(mail)
                setSheetSection(null)
              }}
              swipeable={isMobile}
              onSwipeArchive={isMobile ? quickArchive : undefined}
              onSwipeTrash={isMobile ? quickTrash : undefined}
            />
          ))
        ) : (
          <div className="list-state">
            <strong>{query ? 'No messages found' : 'Search your mail'}</strong>
            <span>
              {query
                ? 'Try a different search or adjust the search options.'
                : 'Type a query in the search bar to search across all your mailboxes.'}
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
            onClick={() => {
              setSheet(null)
              setSheetSection(null)
            }}
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
              {(sheet.folder === 'Trash' || sheet.folder === 'Spam') && (
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
                onClick={() => setSheetSection((current) => (current === 'label' ? null : 'label'))}
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
                onClick={() => setSheetSection((current) => (current === 'move' ? null : 'move'))}
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
          </section>
        </div>
      )}

      {isMobile && advancedOpen && (
        <div className="mobile-sheet" onContextMenu={(event) => event.preventDefault()}>
          <button
            type="button"
            className="mobile-sheet__scrim"
            aria-label="Close search options"
            onClick={closeAdvanced}
          />
          <section
            className="mobile-sheet__panel"
            role="dialog"
            aria-modal="true"
            aria-label="Search options"
          >
            <header className="mobile-sheet__head">
              <div>
                <strong>Search options</strong>
                <span>Narrow your search</span>
              </div>
              <button
                type="button"
                className="icon-button"
                aria-label="Close search options"
                onClick={closeAdvanced}
              >
                <X size={17} />
              </button>
            </header>
            <div className="mobile-sheet__body">
              <AdvancedSearch onSubmit={submitAdvanced} onClose={closeAdvanced} />
            </div>
          </section>
        </div>
      )}

      {isMobile && sortOpen && (
        <div className="mobile-sheet" onContextMenu={(event) => event.preventDefault()}>
          <button
            type="button"
            className="mobile-sheet__scrim"
            aria-label="Close sort menu"
            onClick={() => setSortOpen(false)}
          />
          <section
            className="mobile-sheet__panel"
            role="dialog"
            aria-modal="true"
            aria-label="Sort messages"
          >
            <header className="mobile-sheet__head">
              <div>
                <strong>Sort messages</strong>
                <span>Choose a sort order</span>
              </div>
              <button
                type="button"
                className="icon-button"
                aria-label="Close sort menu"
                onClick={() => setSortOpen(false)}
              >
                <X size={17} />
              </button>
            </header>
            <div className="mobile-sheet__list">
              {sortOptions.map((option) => (
                <button
                  type="button"
                  aria-pressed={sortKey === option.id}
                  key={option.id}
                  onClick={() => handleSort(option.id)}
                >
                  <span>{option.label}</span>
                  {sortKey === option.id && <Check size={16} className="mobile-sheet__selected" />}
                </button>
              ))}
            </div>
          </section>
        </div>
      )}
    </div>
  )
}
