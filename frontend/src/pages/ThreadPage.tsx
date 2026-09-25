import { useCallback, useEffect, useMemo, useState } from 'react'
import { useLocation, useNavigate, useParams } from 'react-router-dom'
import {
  buildForwardDraft,
  buildReplyAllDraft,
  buildReplyDraft,
  filterMails,
  folderFromPath,
  folderPath,
} from '../lib/mail'
import { MailRow } from '../components/MailRow'
import { Reader } from '../components/Reader'
import { useMail } from '../state/mail/MailContext'
import { calendarApi } from '../services/calendar'
import { fetchMailPage, fetchThreadFor, isRemoteMail } from '../services/remote-mail'
import { localIdentity } from '../services/profile'
import type { Mail, ReaderThreadItem } from '../types'
import type { RealtimeEvent } from '../services/ws'

const isEditableTarget = (target: EventTarget | null) =>
  target instanceof HTMLElement &&
  Boolean(target.closest('input, textarea, select, [contenteditable="true"]'))
const emptyThreadItems: ReaderThreadItem[] = []

export function ThreadPage() {
  const navigate = useNavigate()
  const { pathname, state: locationState } = useLocation()
  const { mailId } = useParams()
  const folder = folderFromPath(pathname)
  const {
    mailbox,
    loading,
    openCompose,
    toggleStar,
    markRead,
    markUnread,
    applyAction,
    moveToFolder,
    snooze,
    notify,
  } = useMail()
  const cachedMail = mailbox.find((message) => message.id === mailId)
  const navigationMail = (locationState as { mail?: Mail } | null)?.mail
  const [threadResult, setThreadResult] = useState<{
    mailId: string
    status: 'ready' | 'error'
    items: ReaderThreadItem[]
  } | null>(null)
  const [folderResult, setFolderResult] = useState<{
    folder: string
    rows: Mail[]
    total: number
  } | null>(null)
  const [retry, setRetry] = useState(0)
  const threadItems = threadResult?.mailId === mailId && threadResult?.status === 'ready'
    ? threadResult.items
    : emptyThreadItems
  const threadAnchor = threadItems.find((item) => item.id === mailId)
  const mail: Mail | undefined = useMemo(() => cachedMail ??
    (navigationMail?.id === mailId ? navigationMail : undefined) ??
    (threadAnchor ? {
      id: threadAnchor.id,
      threadId: threadAnchor.threadId,
      initials: threadAnchor.initials,
      sender: threadAnchor.sender,
      email: threadAnchor.email,
      subject: threadAnchor.subject,
      preview: threadAnchor.clearBody.slice(0, 160),
      time: threadAnchor.time,
      label: 'Mail',
      color: threadAnchor.color,
      unread: threadAnchor.seen === false,
      starred: Boolean(threadAnchor.starred),
      folder,
      to: threadAnchor.to,
      cc: threadAnchor.cc,
    } : undefined), [cachedMail, navigationMail, mailId, threadAnchor, folder])

  useEffect(() => {
    let cancelled = false
    void (async () => {
      if (!mailId || !isRemoteMail()) return
      try {
        const items = await fetchThreadFor(mailId)
        if (cancelled) return
        setThreadResult({ mailId, status: 'ready', items })
      } catch {
        if (cancelled) return
        setThreadResult({ mailId, status: 'error', items: [] })
      }
    })()
    return () => {
      cancelled = true
    }
  }, [mailId, retry])
  useEffect(() => {
    if (!isRemoteMail() || !folder) return
    let cancelled = false
    void fetchMailPage(folder, { limit: 50 })
      .then((page) => {
        if (!cancelled) setFolderResult({ folder, rows: page.mails, total: page.total })
      })
      .catch(() => {
        if (!cancelled) setFolderResult(null)
      })
    return () => { cancelled = true }
  }, [folder, retry])
  useEffect(() => {
    if (!isRemoteMail()) return
    const refresh = () => setRetry((value) => value + 1)
    const onRealtime = (incoming: Event) => {
      const detail = (incoming as CustomEvent<RealtimeEvent>).detail
      if (detail?.kind === 'resource-changed' && detail.payload.resource === 'mailbox') refresh()
    }
    window.addEventListener('cs-mail-sent', refresh)
    window.addEventListener('cs-mail-realtime', onRealtime)
    return () => {
      window.removeEventListener('cs-mail-sent', refresh)
      window.removeEventListener('cs-mail-realtime', onRealtime)
    }
  }, [])
  const rows = useMemo(() => {
    if (!mail) return []
    const all = folderResult?.folder === folder
      ? folderResult.rows
      : filterMails(mailbox, folder).slice(0, 50)
    return all.some((item) => item.id === mail.id) ? all : [mail, ...all.slice(0, 29)]
  }, [mailbox, folder, folderResult, mail])
  const total = useMemo(
    () => (mail ? Math.max(1, folderResult?.folder === folder
      ? folderResult.total
      : filterMails(mailbox, folder).length) : 0),
    [mailbox, folder, folderResult, mail],
  )
  const index = rows.findIndex((item) => item.id === mailId)
  const previous = index > 0 ? rows[index - 1] : undefined
  const next = index >= 0 && index < rows.length - 1 ? rows[index + 1] : undefined
  const quickArchive = (message: Mail) => applyAction('archive', [message.id])
  const quickTrash = (message: Mail) => applyAction('trash', [message.id])
  const quickStar = (message: Mail) => toggleStar([message.id])
  const openReply = useCallback((source?: ReaderThreadItem) => {
    if (!mail) return
    const replyMail = source
      ? { ...mail, email: source.email, sender: source.sender, subject: source.subject, to: source.to, cc: source.cc }
      : mail
    openCompose(buildReplyDraft(replyMail, source))
  }, [mail, openCompose])
  const openReplyAll = useCallback((source?: ReaderThreadItem) => {
    if (!mail) return
    const replyMail = source
      ? { ...mail, email: source.email, sender: source.sender, subject: source.subject, to: source.to, cc: source.cc }
      : mail
    openCompose(buildReplyAllDraft(replyMail, localIdentity().email, source))
  }, [mail, openCompose])

  useEffect(() => {
    if (!mail) return
    const handle = (event: KeyboardEvent) => {
      if (isEditableTarget(event.target)) return
      if (event.key.toLowerCase() === 'r') {
        event.preventDefault()
        if (!isRemoteMail() || threadItems.length) openReply(threadItems.at(-1))
      }
      if (event.key.toLowerCase() === 'f') {
        event.preventDefault()
        openCompose(buildForwardDraft(mail))
      }
      if (event.key.toLowerCase() === 's') {
        event.preventDefault()
        toggleStar([mail.id])
      }
      if (event.key.toLowerCase() === 'e') {
        event.preventDefault()
        applyAction('archive', [mail.id])
        navigate(`/mail/${folderPath(folder)}`)
      }
      if (event.key === '#' || event.key.toLowerCase() === 'd') {
        event.preventDefault()
        applyAction('trash', [mail.id])
        navigate(`/mail/${folderPath(folder)}`)
      }
      if (event.key.toLowerCase() === 'j' && next) {
        event.preventDefault()
        navigate(`/mail/${folderPath(folder)}/thread/${next.id}`)
      }
      if (event.key.toLowerCase() === 'k' && previous) {
        event.preventDefault()
        navigate(`/mail/${folderPath(folder)}/thread/${previous.id}`)
      }
    }
    window.addEventListener('keydown', handle)
    return () => window.removeEventListener('keydown', handle)
  }, [mail, folder, navigate, openCompose, toggleStar, applyAction, previous, next, threadItems, openReply])

  if (loading)
    return (
      <div className="list-state">
        <div className="loading-spinner" />
        <strong>Loading conversation</strong>
        <span>Syncing your CS Mail mailbox...</span>
      </div>
    )
  if (!mail && isRemoteMail() && threadResult?.mailId !== mailId)
    return (
      <div className="list-state">
        <div className="loading-spinner" />
        <strong>Loading conversation</strong>
        <span>Finding this message in your mailbox...</span>
      </div>
    )
  if (!mail && isRemoteMail() && threadResult?.status === 'error')
    return (
      <div className="list-state">
        <strong>Could not load this message</strong>
        <span>Try loading the conversation again.</span>
        <button type="button" className="secondary-button" onClick={() => setRetry((value) => value + 1)}>Retry</button>
      </div>
    )
  if (!mail)
    return (
      <div className="list-state">
        <strong>Message not found</strong>
        <span>This conversation may have been removed.</span>
      </div>
    )
  const backToFolder = () => navigate(`/mail/${folderPath(folder)}`)
  const archive = () => {
    applyAction('archive', [mail.id])
    backToFolder()
  }
  const deleteMail = () => {
    applyAction('trash', [mail.id])
    backToFolder()
  }
  const move = (target: string) => {
    moveToFolder([mail.id], target)
    navigate(`/mail/${folderPath(target)}`)
  }
  const snoozeMail = (until: string) => {
    snooze([mail.id], until)
    backToFolder()
  }
  const addToCalendar = () => {
    void calendarApi.createFromMail(mail).catch(() => {})
    notify('Event added to your calendar')
  }
  const goToThread = (item: Mail) => navigate(`/mail/${folderPath(folder)}/thread/${item.id}`, { state: { mail: item } })
  return (
    <div className="split-view">
      <aside className="split-list" aria-label={`${folder} conversations`}>
        <header className="split-list__head">
          <div>
            <p className="eyebrow">{folder}</p>
            <h1>{folder}</h1>
          </div>
          <span>{total} messages</span>
        </header>
        <div className="split-list__rows">
          {rows.map((item) => (
            <MailRow
              key={item.id}
              mail={item}
              active={item.id === mail.id}
              checked={false}
              onClick={goToThread}
              onQuickArchive={quickArchive}
              onQuickStar={quickStar}
              onQuickTrash={quickTrash}
            />
          ))}
          {rows.length === 0 && (
            <div className="list-state">
              <strong>One conversation</strong>
              <span>No other messages in {folder.toLowerCase()}.</span>
            </div>
          )}
        </div>
      </aside>
      <Reader
        key={mail.id}
        mail={mail}
        thread={isRemoteMail() ? (threadResult?.mailId === mail.id ? threadResult.items : []) : undefined}
        threadStatus={isRemoteMail() ? (threadResult?.mailId === mail.id ? threadResult.status : 'loading') : 'demo'}
        onRetryThread={() => {
          setThreadResult(null)
          setRetry((value) => value + 1)
        }}
        onReply={openReply}
        onReplyAll={openReplyAll}
        onForward={() => openCompose(buildForwardDraft(mail))}
        onToggleStar={() => toggleStar([mail.id])}
        onToggleRead={() => (mail.unread ? markRead(mail.id) : markUnread([mail.id]))}
        onBack={backToFolder}
        onArchive={archive}
        onDelete={deleteMail}
        onMove={move}
        onSnooze={snoozeMail}
        onSpam={() => move('Spam')}
        onPrint={() => window.print()}
        onAddToCalendar={addToCalendar}
        onPrevious={previous ? () => goToThread(previous) : undefined}
        onNext={next ? () => goToThread(next) : undefined}
        canPrevious={Boolean(previous)}
        canNext={Boolean(next)}
      />
    </div>
  )
}
