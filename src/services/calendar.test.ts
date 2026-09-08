import { beforeEach, describe, expect, it } from 'vitest'
import { calendarApi, localDate, type CalendarEvent } from './calendar'
import type { Mail } from '../types'

const blank = (patch: Partial<CalendarEvent> = {}): Omit<CalendarEvent, 'id'> => ({
  title: 'Standup',
  date: '2026-09-15',
  allDay: false,
  start: '09:00',
  end: '09:30',
  description: '',
  location: '',
  category: 'work',
  invitees: [],
  ...patch,
})

beforeEach(() => localStorage.clear())

describe('calendarApi events', () => {
  it('seeds demo events including ones for today', () => {
    const today = calendarApi.listOn(localDate(new Date()))
    expect(calendarApi.list().length).toBeGreaterThanOrEqual(6)
    expect(today.length).toBeGreaterThanOrEqual(2)
  })

  it('adds an event and returns the updated collection', () => {
    const next = calendarApi.add(blank())
    expect(next).toHaveLength(calendarApi.list().length)
    expect(calendarApi.list().some(event => event.title === 'Standup' && event.date === '2026-09-15')).toBe(true)
  })

  it('lists only events on the given date, all-day first', () => {
    const allDay = calendarApi.add(blank({ title: 'Invoice', date: '2026-09-15', allDay: true, start: '', end: '' }))
    const timed = calendarApi.add(blank({ title: 'Review', date: '2026-09-15', start: '14:00', end: '15:00' }))
    expect(allDay).toBeDefined()
    const listed = calendarApi.listOn('2026-09-15')
    expect(listed.map(item => item.title)).toEqual(['Invoice', 'Review'])
    expect(timed).toBeDefined()
  })

  it('updates an event in place', () => {
    const created = calendarApi.add(blank())
    const withId = calendarApi.list().find(event => event.id === created[created.length - 1]?.id)
    if (!withId) throw new Error('missing event')
    calendarApi.update(withId.id, { ...withId, title: 'Catch-up' })
    const updated = calendarApi.list().find(event => event.id === withId.id)
    expect(updated?.title).toBe('Catch-up')
  })

  it('removes an event', () => {
    const created = calendarApi.add(blank())
    const withId = calendarApi.list().find(event => event.id === created[created.length - 1]?.id)
    if (!withId) throw new Error('missing event')
    expect(calendarApi.remove(withId.id).some(event => event.id === withId.id)).toBe(false)
  })

  it('persists added events across reloads', () => {
    calendarApi.add(blank({ title: 'Persistent' }))
    expect(calendarApi.list().some(event => event.title === 'Persistent')).toBe(true)
  })
})

describe('calendarApi from mail', () => {
  it('creates a today event from a message subject and sender', () => {
    const mail: Mail = {
      id: 'm1', initials: 'NS', sender: 'Nora Salas', email: 'nora@harbor.co',
      subject: 'Launch readiness', preview: 'Everything is on track',
      time: '', label: 'Inbox', color: 'blue', unread: false,
    }
    const event = calendarApi.createFromMail(mail)
    expect(calendarApi.list().some(item => item.id === event.id)).toBe(true)
    expect(event.title).toBe('Launch readiness')
    expect(event.date).toBe(localDate(new Date()))
    expect(event.description).toContain('nora@harbor.co')
    expect(event.description).toContain('Everything is on track')
  })
})

describe('calendarApi ICS', () => {
  it('exports a timed event with date and time markers', () => {
    const ics = calendarApi.icsExport({ ...blank(), id: 'x' } as CalendarEvent)
    expect(ics).toContain('BEGIN:VCALENDAR')
    expect(ics).toContain('SUMMARY:Standup')
    expect(ics).toContain('DTSTART:20260915T090000')
    expect(ics).toContain('DTEND:20260915T093000')
    expect(ics).toContain('UID:x@harbor.co')
  })

  it('exports an all-day event using VALUE=DATE', () => {
    const ics = calendarApi.icsExport({ ...blank(), id: 'y', allDay: true, start: '', end: '' } as CalendarEvent)
    expect(ics).toContain('DTSTART;VALUE=DATE:20260915')
    expect(ics).toContain('DTEND;VALUE=DATE:20260915')
  })

  it('escapes commas and semicolons in summary text', () => {
    const ics = calendarApi.icsExport({ ...blank(), id: 'z', title: 'Sync, Q3; review' } as CalendarEvent)
    expect(ics).toContain('SUMMARY:Sync\\, Q3\\; review')
  })

  it('imports timed and all-day events from an ICS file', () => {
    const ics = [
      'BEGIN:VCALENDAR',
      'VERSION:2.0',
      'BEGIN:VEVENT',
      'UID:1@example.com',
      'DTSTART:20261005T103000',
      'DTEND:20261005T113000',
      'SUMMARY:Import demo',
      'LOCATION:Zoom',
      'DESCRIPTION:Notes \\n more',
      'END:VEVENT',
      'BEGIN:VEVENT',
      'UID:2@example.com',
      'DTSTART;VALUE=DATE:20261006',
      'SUMMARY:Birthday',
      'END:VEVENT',
      'END:VCALENDAR',
    ].join('\r\n')
    const imported = calendarApi.icsImport(ics)
    expect(imported).toHaveLength(2)
    expect(imported[0]).toMatchObject({ title: 'Import demo', date: '2026-10-05', start: '10:30', end: '11:30', location: 'Zoom', description: 'Notes \n more' })
    expect(imported[1]).toMatchObject({ title: 'Birthday', date: '2026-10-06', allDay: true, start: '' })
  })

  it('ignores text without calendar events', () => {
    expect(calendarApi.icsImport('no events here')).toHaveLength(0)
  })
})