import { Archive, CalendarClock, Clock3, FilePenLine, Inbox as InboxIcon, Mail as MailIcon, MailOpen, ShieldAlert, Send, Star, Trash2 } from 'lucide-react'
import { foldersApi } from '../services/folders'
import type { Draft, Mail, Mailbox } from '../types'

type NavGroup = 'mail' | 'compose' | 'more'
type FolderNav = { label: Mailbox; icon: typeof InboxIcon; group: NavGroup }

export const folders: FolderNav[] = [
  { label: 'Inbox', icon: InboxIcon, group: 'mail' },
  { label: 'Unread', icon: MailIcon, group: 'mail' },
  { label: 'Starred', icon: Star, group: 'mail' },
  { label: 'Snoozed', icon: Clock3, group: 'mail' },
  { label: 'Sent', icon: Send, group: 'compose' },
  { label: 'Scheduled', icon: CalendarClock, group: 'compose' },
  { label: 'Drafts', icon: FilePenLine, group: 'compose' },
  { label: 'All Mail', icon: MailOpen, group: 'more' },
  { label: 'Archive', icon: Archive, group: 'more' },
  { label: 'Spam', icon: ShieldAlert, group: 'more' },
  { label: 'Trash', icon: Trash2, group: 'more' },
]

export const navGroupTitles: Record<NavGroup, string> = {
  mail: 'Mail',
  compose: 'Compose',
  more: 'All folders',
}

export const folderSlug: Record<Mailbox, string> = {
  Inbox: 'inbox',
  Unread: 'unread',
  Starred: 'starred',
  Snoozed: 'snoozed',
  Sent: 'sent',
  Scheduled: 'scheduled',
  Drafts: 'drafts',
  'All Mail': 'all',
  Archive: 'archive',
  Spam: 'spam',
  Trash: 'trash',
}

export const folderFromPath = (path: string): string => {
  const customMatch = path.match(/\/mail\/folders\/([^/]+)/)
  if (customMatch) {
    const custom = foldersApi.byId(decodeURIComponent(customMatch[1]))
    return custom?.name ?? 'Inbox'
  }
  const match = path.match(/\/mail\/([^/]+)/)
  const slug = match?.[1] || 'inbox'
  return folders.find(item => folderSlug[item.label] === slug)?.label || 'Inbox'
}

export const isCustomFolder = (folder: string): boolean => {
  if (folder in folderSlug) return false
  return Boolean(foldersApi.byName(folder))
}

export const folderPath = (folder: string): string => {
  const slug = folderSlug[folder as Mailbox]
  if (slug) return slug
  return foldersApi.byName(folder)?.id ? `folders/${foldersApi.byName(folder)?.id}` : 'inbox'
}

export const categoryLabels: Record<string, string[]> = {
  Primary: ['Clients', 'Important'],
  Promotions: ['Finance'],
  Social: ['Internal'],
  Updates: ['Attachment'],
}

const folderCount = (mailbox: Mail[], folder: Mailbox) =>
  mailbox.filter(mail => (mail.folder || 'Inbox') === folder).length

export const getFolderCounts = (mailbox: Mail[]): Partial<Record<Mailbox, number>> => ({
  Inbox: folderCount(mailbox, 'Inbox'),
  Unread: mailbox.filter(mail => mail.unread && mail.folder !== 'Trash').length,
  Starred: mailbox.filter(mail => mail.starred && mail.folder !== 'Trash').length,
  Snoozed: folderCount(mailbox, 'Snoozed'),
  Sent: folderCount(mailbox, 'Sent'),
  Drafts: folderCount(mailbox, 'Drafts'),
  'All Mail': mailbox.filter(mail => mail.folder !== 'Trash').length,
  Archive: folderCount(mailbox, 'Archive'),
  Spam: folderCount(mailbox, 'Spam'),
  Trash: folderCount(mailbox, 'Trash'),
})

export const buildThread = (mail: Mail) => [
  {
    sender: mail.sender,
    email: mail.email,
    initials: mail.initials,
    color: mail.color,
    copy: `${mail.preview}\n\nI wanted to make sure “${mail.subject}” crossed your desk before the next sync call. Let me know if anything is missing.`,
    time: mail.time,
  },
  {
    sender: 'Alex Morgan',
    email: 'alex@harbor.co',
    initials: 'AM',
    color: 'teal',
    copy: `On ${mail.time}, ${mail.sender} <${mail.email}> wrote:\n\n${mail.preview}\n\nThanks for sending this over — I have worked through the points and the notes look ready for next steps.`,
    time: 'Today',
  },
  {
    sender: mail.sender,
    email: mail.email,
    initials: mail.initials,
    color: mail.color,
    copy: `Perfect — I have folded your feedback into the latest version of “${mail.subject}” and will flag anything that needs a decision.`,
    time: 'Yesterday',
  },
]

const searchTerms = (query: string) => query.trim().toLowerCase().split(/\s+/).filter(Boolean)

const inScopeFolder = (scope: string): string | 'any' | null => {
  const map: Record<string, string> = {
    inbox: 'Inbox',
    sent: 'Sent',
    trash: 'Trash',
    archive: 'Archive',
    spam: 'Spam',
    drafts: 'Drafts',
    snoozed: 'Snoozed',
    starred: 'Starred',
    unread: 'Unread',
    all: 'All Mail',
  }
  if (scope === 'any') return 'any'
  return map[scope] ?? null
}

export const matchesSearch = (mail: Mail, query: string) => searchTerms(query).every(term => {
  const negate = term.startsWith('-')
  const base = negate ? term.slice(1) : term
  const textFields = `${mail.sender} ${mail.email} ${mail.subject} ${mail.preview} ${mail.label} ${(mail.to ?? []).join(' ')}`.toLowerCase()
  let result = textFields.includes(base)
  if (base.startsWith('from:')) result = `${mail.sender} ${mail.email}`.toLowerCase().includes(base.slice(5))
  else if (base.startsWith('to:')) result = (mail.to ?? []).some(recipient => recipient.toLowerCase().includes(base.slice(4)))
  else if (base.startsWith('subject:')) result = mail.subject.toLowerCase().includes(base.slice(8))
  else if (base.startsWith('label:')) result = mail.label.toLowerCase() === base.slice(6)
  else if (base.startsWith('in:')) { const scoped = inScopeFolder(base.slice(3)); result = scoped === 'any' || scoped === 'All Mail' ? mail.folder !== 'Trash' : (mail.folder || 'Inbox') === scoped }
  else if (base === 'is:unread') result = mail.unread
  else if (base === 'is:read') result = !mail.unread
  else if (base === 'is:starred') result = Boolean(mail.starred)
  else if (base === 'is:snoozed') result = mail.folder === 'Snoozed'
  else if (base === 'is:sent') result = mail.folder === 'Sent'
  else if (base === 'is:draft') result = mail.folder === 'Drafts'
  else if (base === 'has:attachment') result = Boolean(mail.attachment)
  return negate ? !result : result
})

export const filterMails = (mailbox: Mail[], folder: string, query = '', category?: string): Mail[] => {
  const scope = query.match(/\bin:([a-zA-Z-]+)\b/)?.[1]?.toLowerCase()
  const scoped = scope ? inScopeFolder(scope) : null
  const effectiveFolder = scoped && scoped !== 'any' ? scoped : (scoped === 'any' ? 'All Mail' : folder)
  return mailbox.filter(mail => (
    matchesSearch(mail, query) &&
    (effectiveFolder === 'Inbox' && category && category !== 'Primary' ? categoryLabels[category]?.includes(mail.label) : true) &&
    (effectiveFolder === 'All Mail' ? mail.folder !== 'Trash'
      : effectiveFolder === 'Unread' ? mail.unread && mail.folder !== 'Trash'
        : effectiveFolder === 'Starred' ? mail.starred && mail.folder !== 'Trash'
          : effectiveFolder === 'Sent' ? mail.folder === 'Sent'
            : (mail.folder || 'Inbox') === effectiveFolder)
  ))
}

const prefixSubject = (mail: Mail, prefix: 'Re:' | 'Fwd:') =>
  mail.subject.toLowerCase().startsWith(prefix.toLowerCase().replace(':', '')) ? mail.subject : `${prefix} ${mail.subject}`

export const selfEmail = 'alex@harbor.co'

export const parseAddresses = (value: string): string[] => value.split(',').map(part => part.trim()).filter(Boolean)

export const buildReplyDraft = (mail: Mail): Partial<Draft> => ({
  to: mail.email,
  subject: prefixSubject(mail, 'Re:'),
  body: '',
})

export const buildReplyAllDraft = (mail: Mail): Partial<Draft> => ({
  to: [mail.email, ...(mail.to ?? []).filter(recipient => recipient.toLowerCase() !== selfEmail.toLowerCase())].join(', '),
  cc: (mail.cc ?? []).filter(recipient => recipient.toLowerCase() !== selfEmail.toLowerCase()).join(', '),
  subject: prefixSubject(mail, 'Re:'),
  body: '',
})

export const buildForwardDraft = (mail: Mail): Partial<Draft> => ({
  to: '',
  subject: prefixSubject(mail, 'Fwd:'),
  body: `\n\n\n---------- Forwarded message ----------\nFrom: ${mail.sender} <${mail.email}>\nSubject: ${mail.subject}\nDate: ${mail.time}\n\n${mail.preview}`,
  attachments: mail.attachment && mail.attachmentName ? [mail.attachmentName] : [],
})

export const buildSentMail = (draft: Draft, time = 'Just now'): Mail => {
  const from = draft.from
  const sender = from?.name ?? 'Alex Morgan'
  const email = from?.email ?? 'alex@harbor.co'
  const initials = sender.split(/\s+/).map(part => part[0]).slice(0, 2).join('').toUpperCase() || 'AM'
  return {
    id: `local-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`,
    initials,
    sender,
    email,
    subject: draft.subject || '(no subject)',
    preview: draft.body.slice(0, 120) || (draft.attachments.length ? 'Attached file' : '(no body)'),
    time,
    label: 'Sent',
    color: 'teal',
    unread: false,
    folder: 'Sent',
    to: parseAddresses(draft.to),
    attachment: draft.attachments.length > 0,
  }
}

export type SnoozeOption = 'later-today' | 'tomorrow' | 'this-weekend' | 'next-week'

export const snoozeOptions: { id: SnoozeOption; label: string }[] = [
  { id: 'later-today', label: 'Later today' },
  { id: 'tomorrow', label: 'Tomorrow' },
  { id: 'this-weekend', label: 'This weekend' },
  { id: 'next-week', label: 'Next week' },
]

const nextWeekday = (date: Date, weekday: number) => {
  const next = new Date(date)
  next.setDate(date.getDate() + ((weekday - date.getDay() + 7) % 7 || 7))
  return next
}

export const snoozeAt = (option: SnoozeOption, now = new Date()): string => {
  const atHour = (date: Date, hour: number) => { const target = new Date(date); target.setHours(hour, 0, 0, 0); return target }
  if (option === 'later-today') {
    if (atHour(now, 17).getTime() > now.getTime()) return atHour(now, 17).toISOString()
    return atHour(new Date(now.getTime() + 24 * 60 * 60 * 1000), 8).toISOString()
  }
  if (option === 'tomorrow') return atHour(new Date(now.getTime() + 24 * 60 * 60 * 1000), 8).toISOString()
  if (option === 'this-weekend') return atHour(nextWeekday(now, 6), 8).toISOString()
  return atHour(nextWeekday(now, 1), 8).toISOString()
}