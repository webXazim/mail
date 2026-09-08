import { useEffect, useMemo } from 'react'
import { useLocation, useNavigate, useParams } from 'react-router-dom'
import { buildForwardDraft, buildReplyAllDraft, buildReplyDraft, filterMails, folderFromPath, folderPath } from '../lib/mail'
import { MailRow } from '../components/MailRow'
import { Reader } from '../components/Reader'
import { useMail } from '../state/mail/MailContext'
import type { Mail } from '../types'

const isEditableTarget = (target: EventTarget | null) =>
  target instanceof HTMLElement && Boolean(target.closest('input, textarea, select, [contenteditable="true"]'))

export function ThreadPage() {
  const navigate = useNavigate()
  const { pathname } = useLocation()
  const { mailId } = useParams()
  const folder = folderFromPath(pathname)
  const { mailbox, loading, openCompose, toggleStar, markRead, markUnread, applyAction, moveToFolder, snooze } = useMail()
  const mail = mailbox.find(message => message.id === mailId)
  const rows = useMemo(() => {
    if (!mail) return []
    const all = filterMails(mailbox, folder).slice(0, 30)
    return all.some(item => item.id === mail.id) ? all : [mail, ...all.slice(0, 29)]
  }, [mailbox, folder, mail])
  const total = useMemo(() => (mail ? filterMails(mailbox, folder).length : 0), [mailbox, folder, mail])
  const index = rows.findIndex(item => item.id === mailId)
  const previous = index > 0 ? rows[index - 1] : undefined
  const next = index >= 0 && index < rows.length - 1 ? rows[index + 1] : undefined
  const quickArchive = (message: Mail) => applyAction('archive', [message.id])
  const quickTrash = (message: Mail) => applyAction('trash', [message.id])
  const quickStar = (message: Mail) => toggleStar([message.id])

  useEffect(() => {
    if (!mail) return
    const handle = (event: KeyboardEvent) => {
      if (isEditableTarget(event.target)) return
      if (event.key.toLowerCase() === 'r') { event.preventDefault(); openCompose(buildReplyDraft(mail)) }
      if (event.key.toLowerCase() === 'f') { event.preventDefault(); openCompose(buildForwardDraft(mail)) }
      if (event.key.toLowerCase() === 's') { event.preventDefault(); toggleStar([mail.id]) }
      if (event.key.toLowerCase() === 'e') { event.preventDefault(); applyAction('archive', [mail.id]); navigate(`/mail/${folderPath(folder)}`) }
      if (event.key === '#' || event.key.toLowerCase() === 'd') { event.preventDefault(); applyAction('trash', [mail.id]); navigate(`/mail/${folderPath(folder)}`) }
      if (event.key.toLowerCase() === 'j' && next) { event.preventDefault(); navigate(`/mail/${folderPath(folder)}/thread/${next.id}`) }
      if (event.key.toLowerCase() === 'k' && previous) { event.preventDefault(); navigate(`/mail/${folderPath(folder)}/thread/${previous.id}`) }
    }
    window.addEventListener('keydown', handle)
    return () => window.removeEventListener('keydown', handle)
  }, [mail, folder, navigate, openCompose, toggleStar, applyAction, previous, next])

  if (loading) return <div className="list-state"><div className="loading-spinner" /><strong>Loading conversation</strong><span>Syncing your Harbor Mailbox...</span></div>
  if (!mail) return <div className="list-state"><strong>Message not found</strong><span>This conversation may have been removed.</span></div>
  const backToFolder = () => navigate(`/mail/${folderPath(folder)}`)
  const archive = () => { applyAction('archive', [mail.id]); backToFolder() }
  const deleteMail = () => { applyAction('trash', [mail.id]); backToFolder() }
  const move = (target: string) => { moveToFolder([mail.id], target); navigate(`/mail/${folderPath(target)}`) }
  const snoozeMail = (until: string) => { snooze([mail.id], until); backToFolder() }
  const goToThread = (item: Mail) => navigate(`/mail/${folderPath(folder)}/thread/${item.id}`)
  return (
    <div className="split-view">
      <aside className="split-list" aria-label={`${folder} conversations`}>
        <header className="split-list__head">
          <div><p className="eyebrow">{folder}</p><h1>{folder}</h1></div>
          <span>{total} messages</span>
        </header>
        <div className="split-list__rows">
          {rows.map(item => (
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
          {rows.length === 0 && <div className="list-state"><strong>One conversation</strong><span>No other messages in {folder.toLowerCase()}.</span></div>}
        </div>
      </aside>
      <Reader mail={mail} onReply={() => openCompose(buildReplyDraft(mail))} onReplyAll={() => openCompose(buildReplyAllDraft(mail))} onForward={() => openCompose(buildForwardDraft(mail))} onToggleStar={() => toggleStar([mail.id])} onToggleRead={() => mail.unread ? markRead(mail.id) : markUnread([mail.id])} onBack={backToFolder} onArchive={archive} onDelete={deleteMail} onMove={move} onSnooze={snoozeMail} onSpam={() => move('Spam')} onPrint={() => window.print()} onPrevious={previous ? () => goToThread(previous) : undefined} onNext={next ? () => goToThread(next) : undefined} canPrevious={Boolean(previous)} canNext={Boolean(next)} />
    </div>
  )
}