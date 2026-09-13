import type { Draft } from '../types'
import type { Template } from '../services/templates'

export const expandTemplate = (body: string, recipientName = ''): string =>
  body.replace(/\{name\}/g, recipientName.trim() || 'there')

export const recipientNameFrom = (recipients: string): string => {
  const first = recipients.split(',')[0]?.trim() ?? ''
  const email = first.match(/\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b/)?.[0] ?? ''
  const display = first.replace(/<[^>]+>/g, '').trim()
  if (display && display !== email) return display
  return email ? (email.split('@')[0] ?? '') : display
}

export const applyTemplate = (draft: Draft, template: Template, recipientName = ''): Draft => {
  const name = recipientName || recipientNameFrom(draft.to || template.to)
  const expanded = expandTemplate(template.body, name)
  const nextBody = draft.body.trim() ? `${draft.body}\n\n${expanded}` : expanded
  return {
    ...draft,
    to: draft.to || template.to,
    cc: draft.cc || template.cc,
    subject: draft.subject || template.subject,
    body: nextBody,
  }
}
