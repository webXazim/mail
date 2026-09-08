import { rulesApi, type RuleAction, type RuleCondition } from '../services/rules'
import { spamApi } from '../services/spam'
import type { Mail } from '../types'

export type PipelineReport = { forwarded: { address: string; count: number }[] }

const protectedFolders = ['Sent', 'Drafts', 'Scheduled']

const isProtected = (mail: Mail) => protectedFolders.includes(mail.folder || '')

const monthStart: Record<string, number> = { jan: 0, feb: 1, mar: 2, apr: 3, may: 4, jun: 5, jul: 6, aug: 7, sep: 8, oct: 9, nov: 10, dec: 11 }

export const estimatedSizeKb = (mail: Mail): number => {
  const textBytes = (mail.subject + mail.preview).length * 2 + (mail.to ?? []).join(',').length * 2
  const attachmentBytes = mail.attachment ? 2048 + (mail.attachmentName?.length ?? 32) * 8 : 0
  return Math.max(1, Math.ceil((textBytes + attachmentBytes) / 1024))
}

const parsedSentDate = (mail: Mail): Date | null => {
  const t = mail.time.toLowerCase().trim()
  const now = new Date()
  if (t === 'just now' || t === 'now') return now
  const ago = t.match(/(\d+)\s*(m|h)\s*ago/)
  if (ago) {
    const minutes = ago[2] === 'h' ? Number(ago[1]) * 60 : Number(ago[1])
    return new Date(now.getTime() - minutes * 60_000)
  }
  const mer = t.match(/^(\d{1,2}):(\d{2})\s*(am|pm)$/)
  if (mer) {
    let hours = Number(mer[1]) % 12
    if (mer[3] === 'pm') hours += 12
    const date = new Date()
    date.setHours(hours, Number(mer[2]), 0, 0)
    return date
  }
  if (t === 'yesterday') return new Date(now.getFullYear(), now.getMonth(), now.getDate() - 1)
  const md = t.match(/^(\w{3})\s+(\d{1,2})$/)
  if (md && monthStart[md[1]] !== undefined) return new Date(now.getFullYear(), monthStart[md[1]], Number(md[2]))
  return null
}

const dayKey = (date: Date): string => `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, '0')}-${String(date.getDate()).padStart(2, '0')}`

const dateComparison = (mail: Mail, condition: Extract<RuleCondition, { field: 'date' }>): boolean => {
  const sent = parsedSentDate(mail)
  if (!sent) return false
  const key = dayKey(sent)
  const target = condition.value
  if (!/^\d{4}-\d{2}-\d{2}$/.test(target)) return false
  switch (condition.op) {
    case 'before': return key < target
    case 'after': return key > target
    case 'on': return key === target
    case 'not-on': return key !== target
    case 'on-or-before': return key <= target
    case 'on-or-after': return key >= target
  }
}

const conditionPasses = (mail: Mail, condition: RuleCondition): boolean => {
  switch (condition.field) {
    case 'from':
      return `${mail.sender} ${mail.email}`.toLowerCase().includes(condition.value.toLowerCase())
    case 'to':
      return mail.email.toLowerCase().includes(condition.value.toLowerCase()) ||
        (mail.to ?? []).some(address => address.toLowerCase().includes(condition.value.toLowerCase()))
    case 'subject':
      return mail.subject.toLowerCase().includes(condition.value.toLowerCase())
    case 'hasAttachment':
      return Boolean(mail.attachment)
    case 'size':
      return condition.op === 'larger' ? estimatedSizeKb(mail) > condition.size : estimatedSizeKb(mail) < condition.size
    case 'date':
      return dateComparison(mail, condition)
  }
}

const applyAction = (mail: Mail, action: RuleAction): Mail => {
  switch (action.kind) {
    case 'label':
      return { ...mail, label: action.value }
    case 'move':
      return { ...mail, folder: action.value }
    case 'archive':
      return { ...mail, folder: 'Archive' }
    case 'keep-in-inbox':
      return { ...mail, folder: 'Inbox' }
    case 'mark-read':
      return { ...mail, unread: false }
    case 'mark-starred':
      return { ...mail, starred: true }
    case 'discard':
      return { ...mail, folder: 'Trash' }
    case 'forward':
      return mail
  }
}

export function applyIncomingFilters(mailbox: Mail[]): { mailbox: Mail[]; report: PipelineReport } {
  const spam = spamApi.load()
  const blocked = new Set(spam.blocked)
  const allowed = new Set(spam.allowed)
  const report: PipelineReport = { forwarded: [] }

  let next = mailbox.map(mail => {
    if (isProtected(mail)) return mail
    const sender = mail.email.toLowerCase()
    if (blocked.has(sender)) return { ...mail, folder: 'Spam' }
    if (allowed.has(sender) && (mail.folder || 'Inbox') === 'Spam') return { ...mail, folder: 'Inbox' }
    return mail
  })

  for (const rule of rulesApi.list()) {
    if (!rule.enabled || rule.conditions.length === 0) continue
    const forwardCounts = new Map<string, number>()
    next = next.map(mail => {
      if (isProtected(mail)) return mail
      if (!rule.conditions.every(condition => conditionPasses(mail, condition))) return mail
      for (const action of rule.actions) {
        if (action.kind === 'forward') {
          const target = action.value.toLowerCase()
          forwardCounts.set(target, (forwardCounts.get(target) ?? 0) + 1)
        }
      }
      return rule.actions.reduce<Mail>((result, action) => applyAction(result, action), mail)
    })
    for (const [address, count] of forwardCounts) report.forwarded.push({ address, count })
  }

  return { mailbox: next, report }
}