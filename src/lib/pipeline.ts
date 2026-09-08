import { rulesApi, type RuleAction, type RuleCondition } from '../services/rules'
import { spamApi } from '../services/spam'
import type { Mail } from '../types'

export type PipelineReport = { forwarded: { address: string; count: number }[] }

const protectedFolders = ['Sent', 'Drafts', 'Scheduled']

const isProtected = (mail: Mail) => protectedFolders.includes(mail.folder || '')

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
    case 'date':
      return false
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