import type { Draft, Mail } from '../types'

export type SchedulePreset =
  'this-evening' | 'tomorrow-morning' | 'tomorrow-evening' | 'next-morning'

export const schedulePresets: { id: SchedulePreset; label: string }[] = [
  { id: 'this-evening', label: 'This evening' },
  { id: 'tomorrow-morning', label: 'Tomorrow 8 AM' },
  { id: 'tomorrow-evening', label: 'Tomorrow 5 PM' },
  { id: 'next-morning', label: 'Next weekday 8 AM' },
]

const atHour = (date: Date, hour: number) => {
  const target = new Date(date)
  target.setHours(hour, 0, 0, 0)
  return target
}

const nextWeekday = (date: Date, weekday: number) => {
  const next = new Date(date)
  next.setDate(date.getDate() + ((weekday - date.getDay() + 7) % 7 || 7))
  return next
}

export const suggestSchedule = (preset: SchedulePreset, now = new Date()): string => {
  if (preset === 'this-evening') {
    if (atHour(now, 17).getTime() > now.getTime()) return atHour(now, 17).toISOString()
    return atHour(new Date(now.getTime() + 24 * 60 * 60 * 1000), 8).toISOString()
  }
  if (preset === 'tomorrow-morning')
    return atHour(new Date(now.getTime() + 24 * 60 * 60 * 1000), 8).toISOString()
  if (preset === 'tomorrow-evening')
    return atHour(new Date(now.getTime() + 24 * 60 * 60 * 1000), 17).toISOString()
  return atHour(nextWeekday(now, 1), 8).toISOString()
}

export const toDateTimeLocal = (value: string): string => {
  const date = new Date(value)
  if (Number.isNaN(date.getTime())) return ''
  const pad = (n: number) => String(n).padStart(2, '0')
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}T${pad(date.getHours())}:${pad(date.getMinutes())}`
}

export const scheduleLabel = (at: string): string =>
  new Date(at).toLocaleString([], {
    month: 'short',
    day: 'numeric',
    hour: 'numeric',
    minute: '2-digit',
  })

export const buildScheduledMail = (entry: {
  id: string
  draft: Draft
  at: string
  status?: 'pending' | 'processing' | 'retry' | 'dead'
  error?: string
  attemptCount?: number
  nextAttemptAt?: string
}): Mail => ({
  id: entry.id,
  initials: 'AM',
  sender: 'You',
  email: entry.draft.to || 'Add recipients to send',
  subject: entry.draft.subject || '(no subject)',
  preview:
    entry.status === 'dead'
      ? `Delivery failed — ${entry.error || 'review and retry this scheduled message'}`
      : entry.status === 'retry'
        ? `Retrying delivery — ${entry.error || 'temporary delivery problem'}`
        : entry.status === 'processing'
          ? 'Sending now…'
          : entry.draft.body.slice(0, 120) ||
            (entry.draft.attachments.length ? 'Scheduled draft with attachment' : 'Blank scheduled draft'),
  time: scheduleLabel(entry.at),
  label: entry.status === 'dead' ? 'Failed' : entry.status === 'retry' ? 'Retrying' : 'Scheduled',
  color: entry.status === 'dead' ? 'coral' : 'teal',
  unread: false,
  folder: 'Scheduled',
  to: entry.draft.to
    ? entry.draft.to
        .split(',')
        .map((part) => {
          const match = part.match(/<([^>]+)>/)
          return (match?.[1] ?? part).trim()
        })
        .filter(Boolean)
    : [],
})

const toneColors = ['coral', 'purple', 'blue', 'green', 'orange']

export function buildReplyMail(draft: Draft, knownName?: string): Mail {
  const emails =
    (draft.to + ',' + draft.cc).match(/[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}/g) ?? []
  const email = emails[0]?.toLowerCase() ?? 'someone@crescentsphere.com'
  const name = knownName?.trim() || email.split('@')[0]?.replace(/[._-]+/g, ' ') || 'Someone'
  const display = name.replace(/\b\w/g, (char) => char.toUpperCase())
  const initials =
    display
      .split(/\s+/)
      .map((part) => part[0]?.toUpperCase() ?? '')
      .slice(0, 2)
      .join('') || '?'
  return {
    id: `reply-${Date.now()}`,
    initials,
    sender: display,
    email,
    subject: `Re: ${draft.subject || 'your message'}`,
    preview:
      'Hey Alex — thanks for writing. Got your update and will get back to you with thoughts shortly.',
    time: new Date().toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' }),
    label: 'Inbox',
    color: toneColors[Math.floor(Math.random() * toneColors.length)],
    unread: true,
    to: ['alex@crescentsphere.com'],
  }
}
