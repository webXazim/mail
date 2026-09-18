import type { Mail } from '../types'
import { apiFetch } from '../lib/api'
import { isRemoteMail } from './remote-mail'

export type EventCategory = 'work' | 'meeting' | 'personal' | 'holiday' | 'reminder'

export type EventInvitee = { email: string; status: 'pending' | 'accepted' | 'declined' }

export type CalendarEvent = {
  id: string
  title: string
  date: string
  allDay: boolean
  start: string
  end: string
  description: string
  location: string
  category: EventCategory
  invitees: EventInvitee[]
}

export const eventCategories: { id: EventCategory; label: string }[] = [
  { id: 'work', label: 'Work' },
  { id: 'meeting', label: 'Meeting' },
  { id: 'personal', label: 'Personal' },
  { id: 'holiday', label: 'Holiday' },
  { id: 'reminder', label: 'Reminder' },
]

const calendarKey = 'harbor-mail:calendar'

export const localDate = (date: Date): string => {
  const year = date.getFullYear()
  const month = String(date.getMonth() + 1).padStart(2, '0')
  const day = String(date.getDate()).padStart(2, '0')
  return `${year}-${month}-${day}`
}

export const parseLocalDate = (value: string): Date => {
  const [year, month, day] = value.split('-').map(Number)
  return new Date(year, month - 1, day)
}

const atOffset = (days: number): string => {
  const date = new Date()
  date.setDate(date.getDate() + days)
  return localDate(date)
}

const seed = (): CalendarEvent[] => [
  {
    id: 'ev-standup',
    title: 'Team standup',
    date: atOffset(0),
    allDay: false,
    start: '09:00',
    end: '09:30',
    description: 'Daily sync — blockers and priorities.',
    location: 'Hangouts',
    category: 'work',
    invitees: [
      { email: 'alex@harbor.co', status: 'accepted' },
      { email: 'jonas@harbor.co', status: 'accepted' },
    ],
  },
  {
    id: 'ev-review',
    title: 'Design review',
    date: atOffset(0),
    allDay: false,
    start: '14:00',
    end: '15:00',
    description: 'Walk through the onboarding flows.',
    location: 'Conference Room B',
    category: 'meeting',
    invitees: [{ email: 'nora@harbor.co', status: 'pending' }],
  },
  {
    id: 'ev-sync',
    title: 'Product sync',
    date: atOffset(1),
    allDay: false,
    start: '10:00',
    end: '10:45',
    description: 'Progress and open questions for the quarter.',
    location: 'Meet — product',
    category: 'work',
    invitees: [],
  },
  {
    id: 'ev-lunch',
    title: 'Lunch with Priya',
    date: atOffset(2),
    allDay: false,
    start: '12:30',
    end: '13:30',
    description: '',
    location: 'The Blue Anchor',
    category: 'personal',
    invitees: [{ email: 'priya@harbor.co', status: 'accepted' }],
  },
  {
    id: 'ev-rehearsal',
    title: 'Q3 launch rehearsal',
    date: atOffset(-1),
    allDay: false,
    start: '16:00',
    end: '17:00',
    description: 'Dry run of the launch demo.',
    location: 'Auditorium',
    category: 'meeting',
    invitees: [],
  },
  {
    id: 'ev-invoice',
    title: 'Submit monthly invoice',
    date: atOffset(3),
    allDay: true,
    start: '',
    end: '',
    description: 'Due before the end of the week.',
    location: '',
    category: 'reminder',
    invitees: [],
  },
  {
    id: 'ev-flight',
    title: 'Flight to Berlin',
    date: atOffset(6),
    allDay: false,
    start: '08:30',
    end: '11:30',
    description: 'TXL check-in two hours before.',
    location: 'Tegel Airport',
    category: 'personal',
    invitees: [],
  },
  {
    id: 'ev-kickoff',
    title: 'Client kickoff',
    date: atOffset(9),
    allDay: false,
    start: '11:00',
    end: '12:00',
    description: 'First session with the Meridian account.',
    location: 'Zoom',
    category: 'meeting',
    invitees: [
      { email: 'priya@harbor.co', status: 'accepted' },
      { email: 'meridian@example.com', status: 'pending' },
    ],
  },
]

type EventRow = CalendarEvent

const rowToEvent = (row: EventRow): CalendarEvent => ({
  id: row.id,
  title: row.title,
  date: row.date,
  allDay: Boolean(row.allDay),
  start: row.start ?? '',
  end: row.end ?? '',
  description: row.description ?? '',
  location: row.location ?? '',
  category: row.category,
  invitees: Array.isArray(row.invitees) ? row.invitees : [],
})

const payloadFor = (event: Omit<CalendarEvent, 'id'>) => ({
  title: event.title,
  date: event.date,
  allDay: event.allDay,
  start: event.allDay && !event.start ? '00:00' : event.start,
  end: event.allDay && !event.end ? '23:59' : event.end,
  description: event.description,
  location: event.location,
  category: event.category,
  invitees: event.invitees,
})

const isServerId = (id: string) => !id.startsWith('ev-')

export const calendarApi = {
  /** Synchronous read from the local cache (seeded in demo mode). */
  list(): CalendarEvent[] {
    const fallback = () => (isRemoteMail() ? [] : seed())
    try {
      const raw = localStorage.getItem(calendarKey)
      if (!raw) return fallback()
      const parsed = JSON.parse(raw) as CalendarEvent[]
      return Array.isArray(parsed) ? parsed : fallback()
    } catch {
      return fallback()
    }
  },
  save(next: CalendarEvent[]) {
    localStorage.setItem(calendarKey, JSON.stringify(next))
  },
  /** API-first refresh; falls back to the local cache when offline or in demo mode. */
  async refresh(): Promise<CalendarEvent[]> {
    if (!isRemoteMail()) return this.list()
    try {
      const result = await apiFetch<{ events: EventRow[] }>('/api/calendar/events')
      const next = (result.events ?? []).map(rowToEvent)
      this.save(next)
      return next
    } catch {
      return this.list()
    }
  },
  async add(event: Omit<CalendarEvent, 'id'>): Promise<CalendarEvent[]> {
    if (isRemoteMail()) {
      try {
        const row = await apiFetch<EventRow>('/api/calendar/events', {
          method: 'POST',
          body: JSON.stringify(payloadFor(event)),
        })
        const next = [...this.list(), rowToEvent(row)]
        this.save(next)
        return next
      } catch {
        // Offline: keep the optimistic local event.
      }
    }
    const next = [...this.list(), { ...event, id: `ev-${Date.now()}` }]
    this.save(next)
    return next
  },
  async update(id: string, patch: Omit<CalendarEvent, 'id'>): Promise<CalendarEvent[]> {
    if (isRemoteMail() && isServerId(id)) {
      try {
        const row = await apiFetch<EventRow>(`/api/calendar/events/${encodeURIComponent(id)}`, {
          method: 'PUT',
          body: JSON.stringify(payloadFor(patch)),
        })
        const next = this.list().map((event) => (event.id === id ? rowToEvent(row) : event))
        this.save(next)
        return next
      } catch {
        // Fall through to the local update so the UI stays responsive offline.
      }
    }
    const next = this.list().map((event) => (event.id === id ? { ...patch, id } : event))
    this.save(next)
    return next
  },
  async remove(id: string): Promise<CalendarEvent[]> {
    if (isRemoteMail() && isServerId(id)) {
      try {
        await apiFetch(`/api/calendar/events/${encodeURIComponent(id)}`, { method: 'DELETE' })
      } catch {
        // Fall through to the local removal so the UI stays responsive offline.
      }
    }
    const next = this.list().filter((event) => event.id !== id)
    this.save(next)
    return next
  },
  listOn(date: string) {
    return this.list()
      .filter((event) => event.date === date)
      .sort((a, b) => (a.allDay ? 0 : timeMinutes(a.start)) - (b.allDay ? 0 : timeMinutes(b.start)))
  },
  async createFromMail(mail: Mail): Promise<CalendarEvent> {
    const next = await this.add({
      title: mail.subject || 'From conversation',
      date: atOffset(0),
      allDay: false,
      start: '09:00',
      end: '09:30',
      description: `${mail.sender} <${mail.email}>\n${mail.preview}`,
      location: '',
      category: 'work',
      invitees: [],
    })
    return next[next.length - 1]
  },
  icsExport(event: CalendarEvent): string {
    const fmt = (value: string) => value.replace(/-/g, '')
    const dateLine = (kind: 'START' | 'END', allDay: boolean, time: string) =>
      allDay
        ? `DT${kind};VALUE=DATE:${fmt(event.date)}`
        : `DT${kind}:${fmt(event.date)}T${time.replace(':', '')}00`
    const escape = (value: string) =>
      value
        .replace(/\\/g, '\\\\')
        .replace(/;/g, '\\;')
        .replace(/,/g, '\\,')
        .replace(/\r?\n/g, '\\n')
    const stamp = localDate(new Date()).replace(/-/g, '')
    return [
      'BEGIN:VCALENDAR',
      'VERSION:2.0',
      'PRODID:-//Harbor Mail//Calendar 1.0//EN',
      'BEGIN:VEVENT',
      `UID:${event.id}@harbor.co`,
      `DTSTAMP:${stamp}T000000`,
      dateLine('START', event.allDay, event.start),
      dateLine('END', event.allDay, event.end || event.start),
      `SUMMARY:${escape(event.title)}`,
      event.location ? `LOCATION:${escape(event.location)}` : '',
      event.description ? `DESCRIPTION:${escape(event.description)}` : '',
      'END:VEVENT',
      'END:VCALENDAR',
      '',
    ]
      .filter((line) => line !== '')
      .join('\r\n')
  },
  icsImport(content: string): CalendarEvent[] {
    const lines = content.replace(/\r/g, '').split('\n')
    const events: CalendarEvent[] = []
    let current: Partial<CalendarEvent> | null = null
    const clear = () => {
      if (current && current.title && current.date) {
        events.push({
          id: `ev-import-${Date.now()}-${events.length}`,
          title: current.title,
          date: current.date,
          allDay: Boolean(current.allDay),
          start: current.start ?? '',
          end: current.end ?? '',
          description: current.description ?? '',
          location: current.location ?? '',
          category: 'work',
          invitees: [],
        })
      }
      current = null
    }
    const unescape = (value: string) =>
      value
        .replace(/\\\\/g, '\\')
        .replace(/\\n/g, '\n')
        .replace(/\\(;|,)/g, '$1')
    for (const line of lines) {
      if (line.startsWith('END:VEVENT')) clear()
      if (line.startsWith('BEGIN:VEVENT')) current = {}
      if (!current) continue
      const [rawName, ...rest] = line.split(':')
      const prop = rawName.split(';')[0]
      const value = unescape(rest.join(':')).trim()
      if (prop === 'SUMMARY') current.title = value
      if (prop === 'LOCATION') current.location = value
      if (prop === 'DESCRIPTION') current.description = value
      if (prop === 'DTSTART') {
        if (/;TZID/i.test(line) || value.includes('T')) current.allDay = false
        const dateValue = value.slice(0, 8)
        current.date = `${dateValue.slice(0, 4)}-${dateValue.slice(4, 6)}-${dateValue.slice(6, 8)}`
        const timeValue = value.slice(9, 13).replace(/(\d{2})(\d{2})/, '$1:$2')
        if (timeValue.length === 5) current.start = timeValue
        if (!value.includes('T')) current.allDay = true
      }
      if (prop === 'DTEND') {
        const timeValue = value.slice(9, 13).replace(/(\d{2})(\d{2})/, '$1:$2')
        if (timeValue.length === 5) current.end = timeValue
        if (!value.includes('T') && typeof current.allDay === 'undefined') current.allDay = true
      }
    }
    return events
  },
}

export const timeMinutes = (time: string): number => {
  const [hours, minutes] = time.split(':').map(Number)
  return (hours || 0) * 60 + (minutes || 0)
}
