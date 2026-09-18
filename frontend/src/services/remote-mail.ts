import { apiFetch } from '../lib/api'
import { authApi } from './auth'
import type { Draft, Mail, Mailbox, ReaderThreadItem, SecurityVerdicts } from '../types'

export type RemoteMailbox = {
  id: string
  name: string
  role: string | null
  total: number
  unread: number
}

export type RemoteRecipient = {
  name?: string
  email: string
}

export type RemoteRow = {
  id: string
  thread_id: string
  subject: string | null
  date: string
  received_at: string
  from?: RemoteRecipient
  to?: RemoteRecipient[]
  cc?: RemoteRecipient[]
  preview: string
  read: boolean
  starred: boolean
  has_attachment: boolean
  size?: number
  mailboxes?: Record<string, boolean>
}

export type RemoteCompose = {
  to: RemoteRecipient[]
  cc: RemoteRecipient[]
  bcc: RemoteRecipient[]
  subject: string
  body_text: string
  body_html?: string
  attachments: { filename: string; content_type: string; data_base64: string }[]
  in_reply_to?: string
  references?: string[]
}

export type RemoteDraftSummary = {
  id: string
  subject: string
  snippet: string
  has_attachments: boolean
  updated_at: string
}

/** True when the user has a live backend session (not the offline demo). */
export const isRemoteMail = () => !authApi.isDemo()

let mailboxesCache: RemoteMailbox[] | null = null
const emailThread = new Map<string, string>()
const emailMailbox = new Map<string, string>()
let roleIndex: Record<string, RemoteMailbox> = {}

export const resetMailCache = () => {
  mailboxesCache = null
  emailThread.clear()
  emailMailbox.clear()
  roleIndex = {}
}

const roleFor = (label: Mailbox): string | null => {
  switch (label) {
    case 'Inbox':
      return 'inbox'
    case 'Sent':
      return 'sent'
    case 'Drafts':
      return 'drafts'
    case 'Trash':
      return 'trash'
    case 'Spam':
      return 'junk'
    case 'Archive':
      return 'archive'
    case 'All Mail':
      return 'all'
    default:
      return null
  }
}

const labelForRole = (role: string | null): Mailbox | null => {
  switch (role) {
    case 'inbox':
      return 'Inbox'
    case 'sent':
      return 'Sent'
    case 'drafts':
      return 'Drafts'
    case 'trash':
      return 'Trash'
    case 'junk':
      return 'Spam'
    case 'archive':
      return 'Archive'
    case 'all':
      return 'All Mail'
    default:
      return null
  }
}

export const mailboxForRole = (role: string | null): RemoteMailbox | undefined =>
  role ? roleIndex[role] : undefined

export async function fetchMailboxes(): Promise<RemoteMailbox[]> {
  if (mailboxesCache) return mailboxesCache
  const data = await apiFetch<{ mailboxes: RemoteMailbox[] }>('/api/mail/mailboxes')
  mailboxesCache = data.mailboxes ?? []
  roleIndex = {}
  for (const mb of mailboxesCache) if (mb.role) roleIndex[mb.role] = mb
  return mailboxesCache
}

async function fetchPreviewRows(mailboxId: string, limit = 50): Promise<RemoteRow[]> {
  const query = new URLSearchParams({ mailbox: mailboxId, limit: String(limit) })
  const data = await apiFetch<{ emails: RemoteRow[] }>(`/api/mail/threads?${query}`)
  return data.emails ?? []
}

const palette = ['teal', 'blue', 'purple', 'pink', 'orange', 'green']

const initialsOf = (name: string, email: string): string => {
  const words = name.trim().split(/\s+/).filter(Boolean)
  if (words.length === 0 && email) return email.slice(0, 2).toUpperCase()
  if (words.length === 1) return words[0].slice(0, 2).toUpperCase()
  return words
    .slice(0, 2)
    .map((word) => word[0])
    .join('')
    .toUpperCase()
}

const colorOf = (email: string): string => {
  let hash = 0
  for (let i = 0; i < email.length; i += 1) hash = (hash * 31 + email.charCodeAt(i)) >>> 0
  return palette[hash % palette.length]
}

export const formatTime = (iso: string | null | undefined): string => {
  if (!iso) return ''
  const date = new Date(iso)
  if (Number.isNaN(date.getTime())) return ''
  const now = new Date()
  const sameDay = (a: Date, b: Date) =>
    a.getFullYear() === b.getFullYear() &&
    a.getMonth() === b.getMonth() &&
    a.getDate() === b.getDate()
  if (sameDay(date, now)) {
    const diff = now.getTime() - date.getTime()
    const minutes = Math.floor(diff / 60000)
    if (minutes < 1) return 'Just now'
    if (minutes < 60) return `${minutes}m ago`
    return date.toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' })
  }
  const yesterday = new Date(now)
  yesterday.setDate(now.getDate() - 1)
  if (sameDay(date, yesterday)) return 'Yesterday'
  const short = date.toLocaleDateString([], { month: 'short', day: 'numeric' })
  if (date.getFullYear() !== now.getFullYear()) {
    return `${short} ${date.getFullYear()}`
  }
  return short
}

const formatAddress = (recipient: RemoteRecipient): string => {
  const email = recipient.email?.trim() ?? ''
  const name = recipient.name?.trim() ?? ''
  if (!name) return email
  if (name === email) return email
  return `${name} <${email}>`
}

export function rowToMail(row: RemoteRow, folder: Mailbox, mailboxId?: string): Mail {
  const sender = row.from?.name?.trim() || row.from?.email.split('@')[0] || 'Unknown'
  const email = row.from?.email ?? ''
  if (row.thread_id) emailThread.set(row.id, row.thread_id)
  if (mailboxId) emailMailbox.set(row.id, mailboxId)
  return {
    id: row.id,
    threadId: row.thread_id,
    initials: initialsOf(sender, email),
    sender,
    email,
    subject: row.subject || '(no subject)',
    preview: row.preview || '',
    time: formatTime(row.received_at || row.date),
    label: 'Mail',
    color: colorOf(email),
    unread: !row.read,
    starred: row.starred,
    attachment: row.has_attachment,
    folder,
    to: row.to?.map(formatAddress),
    cc: row.cc?.map(formatAddress),
  }
}

export const emailMailboxFor = (emailId: string): string | null => emailMailbox.get(emailId) ?? null

/** Full mailbox view: latest previews from every system mailbox, labeled so
 *  the existing client-side folder/search views keep working. */
export async function loadMailboxMail(): Promise<Mail[]> {
  await fetchMailboxes()
  const folders: Mailbox[] = ['Inbox', 'Sent', 'Drafts', 'Trash', 'Spam', 'Archive']
  const picks = folders
    .map((label) => ({ label, mailbox: mailboxForRole(roleFor(label)) }))
    .filter((pick): pick is { label: Mailbox; mailbox: RemoteMailbox } => Boolean(pick.mailbox))

  const pages = await Promise.all(
    picks.map((pick) => fetchPreviewRows(pick.mailbox.id, 50).then((rows) => ({ pick, rows }))),
  )
  const withTs: { ts: number; mail: Mail }[] = []
  for (const { pick, rows } of pages) {
    for (const row of rows) {
      const ts = Date.parse(row.received_at || row.date) || 0
      withTs.push({ ts, mail: rowToMail(row, pick.label, pick.mailbox.id) })
    }
  }
  return withTs.sort((a, b) => b.ts - a.ts).map((entry) => entry.mail)
}

export async function searchMail(query: string, limit = 50): Promise<Mail[]> {
  await fetchMailboxes()
  const params = new URLSearchParams({ q: query, limit: String(limit) })
  const data = await apiFetch<{ emails: RemoteRow[] }>(`/api/mail/search?${params}`)
  const rows = data.emails ?? []
  const out: Mail[] = []
  for (const row of rows) {
    let folder: Mailbox = 'Inbox'
    const keys = Object.keys(row.mailboxes ?? {})
    for (const id of keys) {
      const match = mailboxesCache?.find((mb) => mb.id === id)
      const label = match ? labelForRole(match.role) : null
      if (label) {
        folder = label
        break
      }
    }
    out.push(rowToMail(row, folder, keys[0]))
  }
  return out
}

type RawThreadEmail = RemoteRow & {
  from: RemoteRecipient
  to: RemoteRecipient[]
  cc?: RemoteRecipient[]
  body_text?: string
  body_html?: string
  seen?: boolean
  starred?: boolean
  attachments?: { name?: string; blobId?: string; type?: string; size?: number }[]
  'header:Message-ID'?: string
  security?: SecurityVerdicts
}

/** Full conversation for a thread, resolved from either an email id or a
 *  thread id (the list keeps an emailId→threadId index). */
export async function fetchThreadFor(mailId: string): Promise<ReaderThreadItem[]> {
  await fetchMailboxes()
  const threadId = emailThread.get(mailId) ?? mailId
  const data = await apiFetch<{
    thread_id: string
    count: number
    emails: RawThreadEmail[]
  }>(`/api/mail/thread/${encodeURIComponent(threadId)}`)
  const emails = data.emails ?? []
  return emails.map((email) => {
    const sender = email.from?.name?.trim() || email.from?.email.split('@')[0] || 'Unknown'
    const fromEmail = email.from?.email ?? ''
    return {
      id: email.id,
      threadId: email.thread_id,
      sender,
      email: fromEmail,
      initials: initialsOf(sender, fromEmail),
      color: colorOf(fromEmail),
      copy: email.body_text?.trim() || email.preview || '',
      bodyHtml: email.body_html || '',
      time: formatTime(email.received_at || email.date),
      to: email.to?.map(formatAddress),
      cc: email.cc?.map(formatAddress),
      attachments: email.attachments?.map((part) => ({
        name: part.name ?? 'attachment',
        blobId: part.blobId ?? '',
        type: part.type ?? 'application/octet-stream',
      })),
      messageId: email['header:Message-ID'],
      security: email.security,
    }
  })
}

export const attachmentBlobUrl = (blobId: string) =>
  `/api/mail/attachment/${encodeURIComponent(blobId)}`

export async function setRead(emailIds: string[], read: boolean): Promise<void> {
  if (!emailIds.length) return
  await apiFetch('/api/mail/threads/state', {
    method: 'POST',
    body: JSON.stringify({ emailIds, read }),
  })
}

export async function setStarred(emailIds: string[], starred: boolean): Promise<void> {
  if (!emailIds.length) return
  await apiFetch('/api/mail/threads/state', {
    method: 'POST',
    body: JSON.stringify({ emailIds, starred }),
  })
}

export async function moveEmails(
  emailIds: string[],
  toFolder: Mailbox,
  fromMailboxId?: string | null,
): Promise<void> {
  if (!emailIds.length) return
  await fetchMailboxes()
  const to = mailboxForRole(roleFor(toFolder))
  if (!to) return
  const body: Record<string, unknown> = { emailIds, toMailbox: to.id }
  if (fromMailboxId) body.fromMailbox = fromMailboxId
  await apiFetch('/api/mail/threads/move', { method: 'POST', body: JSON.stringify(body) })
}

export async function destroyEmails(emailIds: string[]): Promise<void> {
  if (!emailIds.length) return
  await apiFetch('/api/mail/threads/destroy', {
    method: 'POST',
    body: JSON.stringify({ emailIds }),
  })
}

export async function emptyTrashRemote(): Promise<void> {
  await fetchMailboxes()
  const trash = mailboxForRole('trash')
  if (!trash) return
  const rows = await fetchPreviewRows(trash.id, 200)
  const ids = rows.map((row) => row.id)
  if (ids.length) await destroyEmails(ids)
}

const parseRecipient = (part: string): RemoteRecipient => {
  const trimmed = part.trim()
  const match = trimmed.match(/^([^<@]+?)\s*<([^>]+)>$/)
  if (match) return { name: match[1].trim() || undefined, email: match[2].trim() }
  return { email: trimmed }
}

export const parseRecipients = (value: string): RemoteRecipient[] =>
  value
    .split(',')
    .map((part) => part.trim())
    .filter(Boolean)
    .map(parseRecipient)

export const recipientsToString = (list: RemoteRecipient[]): string =>
  list.map(formatAddress).join(', ')

export type SendOutcome = {
  ok: boolean
  message_id?: string
  stored?: boolean
}

let activeDraftId: string | null = null

export const remoteDraftApi = {
  activeId: () => activeDraftId,
  setActiveId: (id: string | null) => {
    activeDraftId = id
  },
  clearActive: () => {
    activeDraftId = null
  },

  async list(): Promise<RemoteDraftSummary[]> {
    const data = await apiFetch<{ drafts: RemoteDraftSummary[] }>('/api/drafts')
    return data.drafts ?? []
  },

  async get(id: string): Promise<RemoteCompose> {
    const data = await apiFetch<RemoteCompose & { id: string; updated_at: string }>(
      `/api/drafts/${id}`,
    )
    return {
      to: data.to ?? [],
      cc: data.cc ?? [],
      bcc: data.bcc ?? [],
      subject: data.subject ?? '',
      body_text: data.body_text ?? '',
      body_html: data.body_html,
      attachments: data.attachments ?? [],
    }
  },

  async save(compose: RemoteCompose): Promise<string> {
    let id = activeDraftId
    if (id) {
      await apiFetch(`/api/drafts/${id}`, {
        method: 'PUT',
        body: JSON.stringify(compose),
      })
    } else {
      const created = await apiFetch<{ id: string }>('/api/drafts', {
        method: 'POST',
        body: JSON.stringify(compose),
      })
      id = created.id
      activeDraftId = id
    }
    return id as string
  },

  async remove(id: string): Promise<void> {
    if (activeDraftId === id) activeDraftId = null
    await apiFetch(`/api/drafts/${encodeURIComponent(id)}`, { method: 'DELETE' })
  },
}
/** Replace a stored draft's content entirely (used when re-opening a draft). */
export async function replaceDraftContent(id: string, compose: RemoteCompose): Promise<void> {
  await apiFetch(`/api/drafts/${encodeURIComponent(id)}`, {
    method: 'PUT',
    body: JSON.stringify(compose),
  })
}

/** Map a server draft into a composer Draft. */
export const draftToDraft = (draft: RemoteDraftSummary): Draft => ({
  to: '',
  cc: '',
  bcc: '',
  subject: draft.subject || '',
  body: draft.snippet || '',
  attachments: draft.has_attachments ? [] : [],
  scheduledAt: '',
})

/** Re-open a full server draft in the composer. */
export const composeToDraft = (compose: RemoteCompose): Draft => ({
  to: recipientsToString(compose.to),
  cc: recipientsToString(compose.cc),
  bcc: recipientsToString(compose.bcc),
  subject: compose.subject || '',
  body: compose.body_text || '',
  attachments: [],
  scheduledAt: '',
})

export async function sendCompose(
  compose: RemoteCompose,
  draftId?: string | null,
): Promise<SendOutcome> {
  const body: Record<string, unknown> = { ...compose }
  if (draftId) body.draft_id = draftId
  const result = await apiFetch<SendOutcome>('/api/send', {
    method: 'POST',
    body: JSON.stringify(body),
  })
  remoteDraftApi.clearActive()
  return result
}
