import type { Mail } from '../types'
import { apiFetch } from '../lib/api'
import { isRemoteMail } from './remote-mail'

export type EventCategory = 'work' | 'meeting' | 'personal' | 'holiday' | 'reminder'

export type EventInvitee = { email: string; status: 'pending' | 'accepted' | 'declined' }

export type EventRecurrence = {
  frequency: 'daily' | 'weekly' | 'monthly' | 'yearly'
  interval: number
  until?: string
  count?: number
}

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
  recurrence?: EventRecurrence | null
  timezoneOffsetMinutes?: number
  version?: number
  updatedAt?: string
  seriesId?: string
  seriesStartDate?: string
  occurrenceIndex?: number
}

export const eventCategories: { id: EventCategory; label: string }[] = [
  { id: 'work', label: 'Work' },
  { id: 'meeting', label: 'Meeting' },
  { id: 'personal', label: 'Personal' },
  { id: 'holiday', label: 'Holiday' },
  { id: 'reminder', label: 'Reminder' },
]

const calendarKey = 'cs-mail:calendar'

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
      { email: 'alex@crescentsphere.com', status: 'accepted' },
      { email: 'jonas@crescentsphere.com', status: 'accepted' },
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
    invitees: [{ email: 'nora@crescentsphere.com', status: 'pending' }],
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
    invitees: [{ email: 'priya@crescentsphere.com', status: 'accepted' }],
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
      { email: 'priya@crescentsphere.com', status: 'accepted' },
      { email: 'meridian@example.com', status: 'pending' },
    ],
  },
]

type EventRow = CalendarEvent & {
  version: number
  updatedAt: string
  timezoneOffsetMinutes: number
}

type EventPage = {
  events: EventRow[]
  hasMore: boolean
  nextCursor: string | null
}

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
  recurrence: row.recurrence ?? null,
  timezoneOffsetMinutes: row.timezoneOffsetMinutes ?? 0,
  version: row.version,
  updatedAt: row.updatedAt,
  seriesId: row.seriesId ?? row.id,
  seriesStartDate: row.seriesStartDate ?? row.date,
  occurrenceIndex: row.occurrenceIndex ?? 0,
})

const timezoneOffsetFor = (date: string): number => {
  const target = new Date(`${date}T12:00:00`)
  return -target.getTimezoneOffset()
}

const payloadFor = (event: Omit<CalendarEvent, 'id'>, version?: number) => ({
  title: event.title,
  date: event.date,
  allDay: event.allDay,
  start: event.start,
  end: event.end,
  description: event.description,
  location: event.location,
  category: event.category,
  invitees: event.invitees,
  recurrence: event.recurrence ?? null,
  timezoneOffsetMinutes: event.timezoneOffsetMinutes ?? timezoneOffsetFor(event.date),
  ...(version ? { version } : {}),
})

const isServerId = (id: string) => !id.startsWith('ev-')
const serverIdFor = (event: CalendarEvent) => event.seriesId ?? event.id

const readCache = (): CalendarEvent[] => {
  const fallback = () => (isRemoteMail() ? [] : seed())
  try {
    const raw = localStorage.getItem(calendarKey)
    if (!raw) return fallback()
    const parsed = JSON.parse(raw) as CalendarEvent[]
    return Array.isArray(parsed) ? parsed : fallback()
  } catch {
    return fallback()
  }
}

const saveCache = (events: CalendarEvent[]) => {
  localStorage.setItem(calendarKey, JSON.stringify(events))
}

const remoteRange = async (start?: string, end?: string): Promise<CalendarEvent[]> => {
  const rows: CalendarEvent[] = []
  let cursor: string | null = null
  do {
    const params = new URLSearchParams({ limit: '200' })
    if (start) params.set('start', start)
    if (end) params.set('end', end)
    params.set('timezoneOffsetMinutes', String(-new Date().getTimezoneOffset()))
    if (cursor) params.set('cursor', cursor)
    const page = await apiFetch<EventPage>(`/api/calendar/events?${params}`)
    rows.push(...(page.events ?? []).map(rowToEvent))
    cursor = page.hasMore ? page.nextCursor : null
  } while (cursor)
  return rows
}

export const calendarApi = {
  /** Local storage is only a presentation cache in authenticated mode. */
  list(): CalendarEvent[] {
    return readCache()
  },
  save(next: CalendarEvent[]) {
    saveCache(next)
  },
  async refresh(start?: string, end?: string): Promise<CalendarEvent[]> {
    if (!isRemoteMail()) return this.list()
    const next = await remoteRange(start, end)
    saveCache(next)
    return next
  },
  async add(event: Omit<CalendarEvent, 'id'>): Promise<CalendarEvent[]> {
    if (!isRemoteMail()) {
      const next = [...this.list(), { ...event, id: `ev-${Date.now()}` }]
      saveCache(next)
      return next
    }
    const row = await apiFetch<EventRow>('/api/calendar/events', {
      method: 'POST',
      body: JSON.stringify(payloadFor(event)),
    })
    const next = [...this.list(), rowToEvent(row)]
    saveCache(next)
    return next
  },
  async update(id: string, patch: Omit<CalendarEvent, 'id'>): Promise<CalendarEvent[]> {
    if (!isRemoteMail()) {
      const next = this.list().map((event) => (event.id === id ? { ...patch, id } : event))
      saveCache(next)
      return next
    }
    const current = this.list().find((event) => event.id === id)
    if (!current?.version) throw new Error('Refresh this event before editing it.')
    const targetId = serverIdFor(current)
    if (!isServerId(targetId)) throw new Error('Refresh this event before editing it.')
    const seriesPatch = current.recurrence && current.seriesStartDate
      ? { ...patch, date: current.seriesStartDate }
      : patch
    const row = await apiFetch<EventRow>(`/api/calendar/events/${encodeURIComponent(targetId)}`, {
      method: 'PUT',
      body: JSON.stringify(payloadFor(seriesPatch, current.version)),
    })
    const next = this.list().map((event) =>
      (event.seriesId ?? event.id) === targetId ? rowToEvent(row) : event,
    )
    saveCache(next)
    return next
  },
  async remove(id: string): Promise<CalendarEvent[]> {
    const current = this.list().find((event) => event.id === id)
    const targetId = current ? serverIdFor(current) : id
    if (isRemoteMail()) {
      if (!isServerId(targetId) || !current?.version) throw new Error('Refresh this event before deleting it.')
      await apiFetch(`/api/calendar/events/${encodeURIComponent(targetId)}?version=${encodeURIComponent(String(current.version))}`, { method: 'DELETE' })
    }
    const next = this.list().filter((event) => (event.seriesId ?? event.id) !== targetId)
    saveCache(next)
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
  async exportIcs(event: CalendarEvent): Promise<{ filename: string; content: string }> {
    const targetId = serverIdFor(event)
    if (isRemoteMail() && isServerId(targetId)) {
      return apiFetch<{ filename: string; content: string }>(`/api/calendar/events/${encodeURIComponent(targetId)}/ics`)
    }
    return {
      filename: `${event.title.replace(/[^a-z0-9]+/gi, '-').replace(/^-+|-+$/g, '') || 'event'}.ics`,
      content: this.icsExport(event),
    }
  },
  async importIcs(content: string): Promise<CalendarEvent[]> {
    if (!isRemoteMail()) {
      let next = this.list()
      for (const event of this.icsImport(content)) {
        next = await this.add(event)
      }
      return next
    }
    const result = await apiFetch<{ events: EventRow[] }>('/api/calendar/import', {
      method: 'POST',
      body: JSON.stringify({ content, timezoneOffsetMinutes: -new Date().getTimezoneOffset() }),
    })
    const imported = (result.events ?? []).map(rowToEvent)
    const byId = new Map(this.list().map((event) => [event.id, event]))
    for (const event of imported) byId.set(event.id, event)
    const next = [...byId.values()]
    saveCache(next)
    return next
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
      'PRODID:-//CS Mail//Calendar 1.0//EN',
      'BEGIN:VEVENT',
      `UID:${event.id}@cs-mail`,
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
  icsImport(content: string): Omit<CalendarEvent, 'id'>[] {
    const lines = content.replace(/\r/g, '').split('\n')
    const events: Omit<CalendarEvent, 'id'>[] = []
    let current: Partial<CalendarEvent> | null = null
    const clear = () => {
      if (current?.title && current.date) {
        events.push({
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
      value.replace(/\\\\/g, '\\').replace(/\\n/g, '\n').replace(/\\(;|,)/g, '$1')
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
        current.allDay = !value.includes('T')
        const dateValue = value.slice(0, 8)
        current.date = `${dateValue.slice(0, 4)}-${dateValue.slice(4, 6)}-${dateValue.slice(6, 8)}`
        if (value.includes('T')) current.start = value.slice(9, 13).replace(/(\d{2})(\d{2})/, '$1:$2')
      }
      if (prop === 'DTEND' && value.includes('T')) {
        current.end = value.slice(9, 13).replace(/(\d{2})(\d{2})/, '$1:$2')
      }
    }
    return events
  },
}

export const timeMinutes = (time: string): number => {
  const [hours, minutes] = time.split(':').map(Number)
  return (hours || 0) * 60 + (minutes || 0)
}
