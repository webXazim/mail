import type { Draft, Mail } from '../types'

const toneColors = ['coral', 'purple', 'blue', 'green', 'orange']

export function buildReplyMail(draft: Draft, knownName?: string): Mail {
  const emails = (draft.to + ',' + draft.cc).match(/[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}/g) ?? []
  const email = emails[0]?.toLowerCase() ?? 'someone@harbor.co'
  const name = knownName?.trim() || email.split('@')[0]?.replace(/[._-]+/g, ' ') || 'Someone'
  const display = name.replace(/\b\w/g, char => char.toUpperCase())
  const initials = display.split(/\s+/).map(part => part[0]?.toUpperCase() ?? '').slice(0, 2).join('') || '?'
  return {
    id: `reply-${Date.now()}`,
    initials,
    sender: display,
    email,
    subject: `Re: ${draft.subject || 'your message'}`,
    preview: 'Hey Alex — thanks for writing. Got your update and will get back to you with thoughts shortly.',
    time: new Date().toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' }),
    label: 'Inbox',
    color: toneColors[Math.floor(Math.random() * toneColors.length)],
    unread: true,
    to: ['alex@harbor.co'],
  }
}