import { apiFetch } from '../lib/api'
import { authApi } from './auth'
import { foldersApi } from './folders'
import { labelsApi } from './labels'
import type { Draft, DraftAttachment, Mail, Mailbox, ReaderThreadItem, SecurityVerdicts } from '../types'

export type RemoteMailbox = {
  id: string
  name: string
  role: string | null
  total: number
  unread: number
}

export type RemoteVirtualCounts = { all: number; unread: number; starred: number }

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
  keywords?: Record<string, boolean>
}

export type RemoteCompose = {
  to: RemoteRecipient[]
  cc: RemoteRecipient[]
  bcc: RemoteRecipient[]
  subject: string
  body_text: string
  body_html?: string
  attachments: (DraftAttachment | { id: string })[]
  in_reply_to?: string
  references?: string[]
  identity_id?: string
  client_key?: string
  send_key?: string
  draft_id?: string
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
let mailboxState: string | null = null
let virtualCounts: RemoteVirtualCounts = { all: 0, unread: 0, starred: 0 }
const emailThread = new Map<string, string>()
const emailMailbox = new Map<string, string>()
let roleIndex: Record<string, RemoteMailbox> = {}

export type RemoteMailPage = {
  mails: Mail[]
  hasMore: boolean
  nextAnchor: string | null
  queryState: string | null
  resetRequired: boolean
  total: number
}

export type RemotePageOptions = {
  limit?: number
  anchor?: string | null
  queryState?: string | null
  sort?: 'received_desc' | 'received_asc' | 'sender_asc' | 'sender_desc' | 'subject_asc' | 'subject_desc'
  unread?: boolean
  starred?: boolean
  attachment?: boolean
  mailboxId?: string | null
}

export const resetMailCache = () => {
  mailboxesCache = null
  mailboxState = null
  virtualCounts = { all: 0, unread: 0, starred: 0 }
  emailThread.clear()
  emailMailbox.clear()
  roleIndex = {}
  foldersApi.syncRemote([])
}

export const invalidateMailboxRoster = () => {
  mailboxesCache = null
  mailboxState = null
  roleIndex = {}
}

const roleFor = (label: Mailbox): string | null => {
  switch (label) {
    case 'Inbox': return 'inbox'
    case 'Sent': return 'sent'
    case 'Drafts': return 'drafts'
    case 'Trash': return 'trash'
    case 'Spam': return 'junk'
    case 'Archive': return 'archive'
    default: return null
  }
}

const labelForRole = (role: string | null): Mailbox | null => {
  switch (role) {
    case 'inbox': return 'Inbox'
    case 'sent': return 'Sent'
    case 'drafts': return 'Drafts'
    case 'trash': return 'Trash'
    case 'junk': return 'Spam'
    case 'archive': return 'Archive'
    default: return null
  }
}

export const mailboxForRole = (role: string | null): RemoteMailbox | undefined =>
  role ? roleIndex[role] : undefined

export async function fetchMailboxes(force = false): Promise<RemoteMailbox[]> {
  if (mailboxesCache && !force) return mailboxesCache
  const data = await apiFetch<{
    mailboxes: RemoteMailbox[]
    state?: string | null
    virtual_counts?: Partial<RemoteVirtualCounts>
  }>('/api/mail/mailboxes')
  mailboxesCache = data.mailboxes ?? []
  mailboxState = data.state ?? null
  virtualCounts = {
    all: data.virtual_counts?.all ?? 0,
    unread: data.virtual_counts?.unread ?? 0,
    starred: data.virtual_counts?.starred ?? 0,
  }
  roleIndex = {}
  for (const mb of mailboxesCache) if (mb.role) roleIndex[mb.role] = mb
  foldersApi.syncRemote(
    mailboxesCache
      .filter((mb) => !mb.role)
      .map((mb) => ({ id: mb.id, name: mb.name, total: mb.total, unread: mb.unread })),
  )
  return mailboxesCache
}

export const currentMailboxState = () => mailboxState
export const currentVirtualCounts = (): RemoteVirtualCounts => virtualCounts

const palette = ['teal', 'blue', 'purple', 'pink', 'orange', 'green']

const initialsOf = (name: string, email: string): string => {
  const words = name.trim().split(/\s+/).filter(Boolean)
  if (words.length === 0 && email) return email.slice(0, 2).toUpperCase()
  if (words.length === 1) return words[0].slice(0, 2).toUpperCase()
  return words.slice(0, 2).map((word) => word[0]).join('').toUpperCase()
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
    a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() && a.getDate() === b.getDate()
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
  return date.getFullYear() !== now.getFullYear() ? `${short} ${date.getFullYear()}` : short
}

const formatAddress = (recipient: RemoteRecipient): string => {
  const email = recipient.email?.trim() ?? ''
  const name = recipient.name?.trim() ?? ''
  if (!name || name === email) return email
  return `${name} <${email}>`
}

const automationLabelKeyword = (label: string): string => {
  let slug = 'cs-label-'
  for (const char of label.toLowerCase()) {
    if (/[a-z0-9]/.test(char)) slug += char
    else if ((char === '-' || char === '_' || /\s/.test(char)) && !slug.endsWith('-')) slug += '-'
    if (slug.length >= 56) break
  }
  slug = slug.replace(/-+$/, '')
  return slug === 'cs-label' || slug === 'cs-label-' ? 'cs-label-mail' : slug
}

const labelFromKeywords = (keywords?: Record<string, boolean>): string => {
  const key = Object.keys(keywords ?? {}).find((keyword) => keyword.startsWith('cs-label-'))
  if (!key) return 'Mail'
  const configured = labelsApi.list().find((label) => automationLabelKeyword(label.name) === key)
  if (configured) return configured.name
  return key
    .slice('cs-label-'.length)
    .split('-')
    .filter(Boolean)
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(' ') || 'Mail'
}

const folderFromMailboxIds = (
  row: RemoteRow,
  fallback = 'Inbox',
): { folder: string; mailboxId?: string } => {
  const ids = Object.keys(row.mailboxes ?? {})
  for (const id of ids) {
    const match = mailboxesCache?.find((mb) => mb.id === id)
    if (!match) continue
    const system = labelForRole(match.role)
    if (system) return { folder: system, mailboxId: id }
    if (!match.role) return { folder: match.name, mailboxId: id }
  }
  return { folder: fallback, mailboxId: ids[0] }
}

export function rowToMail(row: RemoteRow, folder?: string, mailboxId?: string): Mail {
  const sender = row.from?.name?.trim() || row.from?.email.split('@')[0] || 'Unknown'
  const email = row.from?.email ?? ''
  if (row.thread_id) emailThread.set(row.id, row.thread_id)
  const inferred = folderFromMailboxIds(row)
  const primaryMailbox = mailboxId ?? inferred.mailboxId
  if (primaryMailbox) emailMailbox.set(row.id, primaryMailbox)
  return {
    id: row.id,
    threadId: row.thread_id,
    initials: initialsOf(sender, email),
    sender,
    email,
    subject: row.subject || '(no subject)',
    preview: row.preview || '',
    time: formatTime(row.received_at || row.date),
    label: labelFromKeywords(row.keywords),
    color: colorOf(email),
    unread: !row.read,
    starred: row.starred,
    attachment: row.has_attachment,
    folder: folder ?? inferred.folder,
    to: row.to?.map(formatAddress),
    cc: row.cc?.map(formatAddress),
  }
}

export const emailMailboxFor = (emailId: string): string | null => emailMailbox.get(emailId) ?? null

const resolvePageScope = async (
  folder: string,
  explicitMailboxId?: string | null,
): Promise<{ scope: string; mailbox?: string }> => {
  await fetchMailboxes()
  if (explicitMailboxId) return { scope: 'mailbox', mailbox: explicitMailboxId }
  if (folder === 'All Mail') return { scope: 'all' }
  if (folder === 'Unread') return { scope: 'unread' }
  if (folder === 'Starred') return { scope: 'starred' }
  const role = roleFor(folder as Mailbox)
  if (role) {
    const mailbox = mailboxForRole(role)
    if (!mailbox && role === 'archive') {
      // Archive is created lazily by the backend when the first message is
      // archived, so an untouched account legitimately starts empty.
      return { scope: 'mailbox', mailbox: '__missing__' }
    }
    return { scope: 'mailbox', mailbox: mailbox?.id }
  }
  const custom = foldersApi.byName(folder) ?? foldersApi.byId(folder)
  return { scope: 'mailbox', mailbox: custom?.id }
}

export async function fetchMailPage(folder: string, options: RemotePageOptions = {}): Promise<RemoteMailPage> {
  const resolved = await resolvePageScope(folder, options.mailboxId)
  if (resolved.scope === 'mailbox' && (!resolved.mailbox || resolved.mailbox === '__missing__')) {
    return { mails: [], hasMore: false, nextAnchor: null, queryState: null, resetRequired: false, total: 0 }
  }
  const query = new URLSearchParams({
    scope: resolved.scope,
    limit: String(options.limit ?? 50),
    sort: options.sort ?? 'received_desc',
  })
  if (resolved.mailbox) query.set('mailbox', resolved.mailbox)
  if (options.anchor) query.set('anchor', options.anchor)
  if (options.queryState) query.set('query_state', options.queryState)
  if (options.unread) query.set('unread', 'true')
  if (options.starred) query.set('starred', 'true')
  if (options.attachment) query.set('attachment', 'true')
  const data = await apiFetch<{
    emails: RemoteRow[]
    has_more: boolean
    next_anchor?: string | null
    query_state?: string | null
    reset_required?: boolean
    total?: number
  }>(`/api/mail/threads?${query}`)
  const fallback = folder === 'All Mail' || folder === 'Unread' || folder === 'Starred' ? undefined : folder
  return {
    mails: (data.emails ?? []).map((row) => rowToMail(row, fallback, resolved.mailbox)),
    hasMore: Boolean(data.has_more),
    nextAnchor: data.next_anchor ?? null,
    queryState: data.query_state ?? null,
    resetRequired: Boolean(data.reset_required),
    total: data.total ?? (data.emails?.length ?? 0),
  }
}

/** Initial hydration is intentionally small. MailListPage fetches subsequent
 * pages from JMAP on demand, so startup cost no longer grows with mailbox size. */
export async function loadMailboxMail(): Promise<Mail[]> {
  await fetchMailboxes(true)
  const page = await fetchMailPage('Inbox', { limit: 50 })
  return page.mails
}

export type RemoteSearchOptions = {
  limit?: number
  anchor?: string | null
  queryState?: string | null
  sort?: RemotePageOptions['sort']
}

export async function searchMail(
  query: string,
  options: RemoteSearchOptions = {},
): Promise<RemoteMailPage> {
  await fetchMailboxes()
  const params = new URLSearchParams({
    q: query,
    limit: String(options.limit ?? 50),
    sort: options.sort ?? 'received_desc',
    tz_offset_minutes: String(-new Date().getTimezoneOffset()),
  })
  if (options.anchor) params.set('anchor', options.anchor)
  if (options.queryState) params.set('query_state', options.queryState)
  const data = await apiFetch<{
    emails: RemoteRow[]
    has_more: boolean
    next_anchor?: string | null
    query_state?: string | null
    reset_required?: boolean
    total?: number
  }>(`/api/mail/search?${params}`)
  return {
    mails: (data.emails ?? []).map((row) => rowToMail(row)),
    hasMore: Boolean(data.has_more),
    nextAnchor: data.next_anchor ?? null,
    queryState: data.query_state ?? null,
    resetRequired: Boolean(data.reset_required),
    total: data.total ?? (data.emails?.length ?? 0),
  }
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

export async function fetchThreadFor(mailId: string): Promise<ReaderThreadItem[]> {
  await fetchMailboxes()
  const threadId = emailThread.get(mailId) ?? mailId
  const data = await apiFetch<{ thread_id: string; count: number; emails: RawThreadEmail[] }>(
    `/api/mail/thread/${encodeURIComponent(threadId)}`,
  )
  return (data.emails ?? []).map((email) => {
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
      clearBody: email.body_text?.trim() || email.preview || '',
      subject: email.subject?.trim() || '(no subject)',
      bodyHtml: email.body_html || '',
      time: formatTime(email.received_at || email.date),
      to: email.to?.map(formatAddress),
      cc: email.cc?.map(formatAddress),
      attachments: email.attachments?.map((part) => ({
        name: part.name ?? 'attachment', blobId: part.blobId ?? '', type: part.type ?? 'application/octet-stream',
      })),
      messageId: email['header:Message-ID'],
      security: email.security,
    }
  })
}

export const attachmentBlobUrl = (blobId: string) => `/api/mail/attachment/${encodeURIComponent(blobId)}`

export async function setRead(emailIds: string[], read: boolean): Promise<void> {
  if (!emailIds.length) return
  await apiFetch('/api/mail/threads/state', { method: 'POST', body: JSON.stringify({ emailIds, read }) })
}

export async function setStarred(emailIds: string[], starred: boolean): Promise<void> {
  if (!emailIds.length) return
  await apiFetch('/api/mail/threads/state', { method: 'POST', body: JSON.stringify({ emailIds, starred }) })
}

export async function moveEmails(
  emailIds: string[],
  toFolder: string,
  fromMailboxes?: Record<string, string>,
): Promise<void> {
  if (!emailIds.length) return
  await fetchMailboxes()
  const role = roleFor(toFolder as Mailbox)
  const custom = !role ? foldersApi.byName(toFolder) ?? foldersApi.byId(toFolder) : undefined
  const body: Record<string, unknown> = { emailIds }
  if (role) body.toRole = role
  else if (custom) body.toMailbox = custom.id
  else throw new Error(`Folder ${toFolder} is not available`)
  if (fromMailboxes && Object.keys(fromMailboxes).length) body.fromMailboxes = fromMailboxes
  await apiFetch('/api/mail/threads/move', { method: 'POST', body: JSON.stringify(body) })
  invalidateMailboxRoster()
}

export async function destroyEmails(emailIds: string[]): Promise<void> {
  if (!emailIds.length) return
  await apiFetch('/api/mail/threads/destroy', { method: 'POST', body: JSON.stringify({ emailIds }) })
  invalidateMailboxRoster()
}

export async function emptyTrashRemote(): Promise<number> {
  await fetchMailboxes()
  const trash = mailboxForRole('trash')
  if (!trash) return 0
  const result = await apiFetch<{ ok: boolean; deleted: number }>(
    `/api/mail/mailboxes/${encodeURIComponent(trash.id)}/empty`,
    { method: 'POST' },
  )
  invalidateMailboxRoster()
  return result.deleted ?? 0
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
  request_id?: string
  idempotency_key?: string
  message_id?: string
  sent_id?: string | null
  stored?: boolean
  deduplicated?: boolean
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
      identity_id: data.identity_id,
      client_key: data.client_key,
      send_key: data.send_key,
      draft_id: data.id,
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
  identityId: compose.identity_id,
  serverDraftId: compose.draft_id,
  clientKey: compose.client_key,
  sendKey: compose.send_key || crypto.randomUUID(),
  attachments: compose.attachments.map((item) => ({
    id: item.id,
    filename: 'filename' in item ? item.filename : 'attachment',
    content_type: 'content_type' in item ? item.content_type : 'application/octet-stream',
    size: 'size' in item ? item.size : 0,
    ...('sha256_hex' in item && item.sha256_hex ? { sha256_hex: item.sha256_hex } : {}),
    ...('status' in item && item.status ? { status: item.status } : {}),
    ...('expires_at' in item && item.expires_at ? { expires_at: item.expires_at } : {}),
  })),
  scheduledAt: '',
})

export async function sendCompose(
  compose: RemoteCompose,
  idempotencyKey: string,
  draftId?: string | null,
): Promise<SendOutcome> {
  const body: Record<string, unknown> = { ...compose }
  if (draftId) body.draft_id = draftId
  const result = await apiFetch<SendOutcome>('/api/send', {
    method: 'POST',
    headers: { 'Idempotency-Key': idempotencyKey },
    body: JSON.stringify(body),
  })
  remoteDraftApi.clearActive()
  return result
}

export async function sendStatus(idempotencyKey: string): Promise<{
  status: 'prepared' | 'submitting' | 'sent' | 'failed' | 'uncertain'
  request_id: string
  message_id: string
  sent_id?: string | null
  last_error?: string
}> {
  return apiFetch(`/api/send/status/${encodeURIComponent(idempotencyKey)}`)
}
