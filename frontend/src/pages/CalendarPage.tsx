import { useEffect, useRef, useState, type FormEvent } from 'react'
import {
  CalendarDays,
  CalendarPlus,
  ChevronLeft,
  ChevronRight,
  Clock3,
  Download,
  FileUp,
  Plus,
  Trash2,
  X,
} from 'lucide-react'
import { useFocusTrap } from '../hooks/useFocusTrap'
import {
  calendarApi,
  eventCategories,
  localDate,
  timeMinutes,
  type CalendarEvent,
  type EventCategory,
} from '../services/calendar'

const weekdayLabels = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat']

const monthsOfYear = (year: number, month: number): Date[] => {
  const first = new Date(year, month, 1)
  const start = new Date(year, month, 1 - first.getDay())
  return Array.from({ length: 42 }, (_, index) => {
    const day = new Date(start)
    day.setDate(start.getDate() + index)
    return day
  })
}

const timeLabel = (event: CalendarEvent) =>
  event.allDay ? 'All day' : `${event.start} – ${event.end}`

const blankDraft = (date: string): Omit<CalendarEvent, 'id'> => ({
  title: '',
  date,
  allDay: false,
  start: '09:00',
  end: '10:00',
  description: '',
  location: '',
  category: 'work',
  invitees: [],
})

export function CalendarPage() {
  const [events, setEvents] = useState<CalendarEvent[]>(() => calendarApi.list())
  const [cursor, setCursor] = useState(() => {
    const now = new Date()
    return new Date(now.getFullYear(), now.getMonth(), 1)
  })
  const [viewDate, setViewDate] = useState(localDate(new Date()))
  const [editor, setEditor] = useState<{
    event: CalendarEvent | null
    draft: Omit<CalendarEvent, 'id'>
  } | null>(null)
  const [notice, setNotice] = useState('')
  const fileRef = useRef<HTMLInputElement>(null)

  useEffect(() => {
    let cancelled = false
    void (async () => {
      const rows = await calendarApi.refresh()
      if (!cancelled) setEvents(rows)
    })()
    return () => {
      cancelled = true
    }
  }, [])

  const year = cursor.getFullYear()
  const month = cursor.getMonth()
  const today = localDate(new Date())
  const cells = monthsOfYear(year, month)
  const dayEvents = events
    .filter((event) => event.date === viewDate)
    .sort((a, b) => (a.allDay ? 0 : timeMinutes(a.start)) - (b.allDay ? 0 : timeMinutes(b.start)))
  const weekRows: Date[][] = []
  for (let index = 0; index < cells.length; index += 7) weekRows.push(cells.slice(index, index + 7))
  const eventsOn = (date: string) =>
    events
      .filter((event) => event.date === date)
      .sort(
        (a, b) => (a.allDay ? -1 : timeMinutes(a.start)) - (b.allDay ? -1 : timeMinutes(b.start)),
      )

  const showNotice = (message: string) => {
    setNotice(message)
    window.setTimeout(() => setNotice(''), 3200)
  }

  const openNew = (date: string) => setEditor({ event: null, draft: blankDraft(date) })
  const openEvent = (event: CalendarEvent) => setEditor({ event, draft: { ...event } })

  const save = async (draft: Omit<CalendarEvent, 'id'>, existing: CalendarEvent | null) => {
    const normalized = { ...draft, title: draft.title.trim() || 'Untitled event' }
    const next = existing
      ? await calendarApi.update(existing.id, normalized)
      : await calendarApi.add(normalized)
    setEvents(next)
    setEditor(null)
    showNotice(existing ? 'Event updated' : 'Event created')
  }

  const remove = async (event: CalendarEvent) => {
    setEvents(await calendarApi.remove(event.id))
    setEditor(null)
    showNotice('Event deleted')
  }

  const downloadIcs = (event: CalendarEvent) => {
    const blob = new Blob([calendarApi.icsExport(event)], { type: 'text/calendar' })
    const url = URL.createObjectURL(blob)
    const link = document.createElement('a')
    link.href = url
    link.download = `${event.title.replace(/[^a-z0-9]+/gi, '-').replace(/^-+|-+$/g, '') || 'event'}.ics`
    link.click()
    URL.revokeObjectURL(url)
    showNotice('Calendar file downloaded')
  }

  const importIcs = async (file: File) => {
    const imported = calendarApi.icsImport(await file.text())
    if (!imported.length) {
      showNotice('No events found in that file')
      return
    }
    let next = calendarApi.list()
    for (const event of imported) next = await calendarApi.add(event)
    setEvents(next)
    showNotice(`Imported ${imported.length} event${imported.length === 1 ? '' : 's'}`)
  }

  const goMonth = (amount: number) => {
    const target = new Date(year, month + amount, 1)
    setCursor(target)
    setViewDate(localDate(target))
  }
  const goToday = () => {
    setCursor((current) => new Date(current.getFullYear(), current.getMonth(), 1))
    setViewDate(localDate(new Date()))
  }

  return (
    <div className="calendar-page">
      <header className="calendar-head">
        <div>
          <p className="eyebrow">Workspace / Calendar</p>
          <h1>{cursor.toLocaleString([], { month: 'long', year: 'numeric' })}</h1>
        </div>
        <div className="calendar-head__actions">
          <input
            ref={fileRef}
            type="file"
            accept=".ics,text/calendar"
            aria-label="Import calendar file"
            hidden
            onChange={(event) => {
              const file = event.target.files?.[0]
              if (file) void importIcs(file)
              event.target.value = ''
            }}
          />
          <button
            type="button"
            className="secondary-button"
            onClick={() => fileRef.current?.click()}
          >
            <FileUp size={15} />
            Import .ics
          </button>
          <button type="button" className="secondary-button" onClick={() => goMonth(1)}>
            <ChevronRight size={15} />
            Next
          </button>
          <button type="button" className="secondary-button" onClick={() => goMonth(-1)}>
            <ChevronLeft size={15} />
            Previous
          </button>
          <button type="button" className="secondary-button" onClick={goToday}>
            Today
          </button>
          <button
            type="button"
            className="primary-button"
            onClick={() => openNew(localDate(new Date()))}
          >
            <CalendarPlus size={16} />
            New event
          </button>
        </div>
      </header>

      <div className="calendar-body">
        <div className="calendar-grid" aria-label="Month view" role="grid">
          <div className="calendar-weekdays" role="row">
            {weekdayLabels.map((day) => (
              <span role="columnheader" key={day}>
                {day}
              </span>
            ))}
          </div>
          {weekRows.map((week, weekIndex) => (
            <div className="calendar-week" role="row" key={weekIndex}>
              {week.map((date) => {
                const key = localDate(date)
                const items = eventsOn(key)
                const isCurrent = date.getMonth() === month
                const isToday = key === today
                return (
                  <div
                    role="gridcell"
                    className={`calendar-day ${isCurrent ? '' : 'calendar-day--other'} ${isToday ? 'calendar-day--today' : ''}`}
                    key={key}
                    onClick={() => openNew(key)}
                  >
                    <span className="calendar-day__num">{date.getDate()}</span>
                    {items.slice(0, 3).map((event) => (
                      <button
                        type="button"
                        className={`calendar-chip calendar-chip--${event.category}`}
                        key={event.id}
                        onClick={(eventClick) => {
                          eventClick.stopPropagation()
                          openEvent(event)
                        }}
                        title={event.title}
                      >
                        <span>{event.allDay ? '•' : event.start || ''}</span>
                        <strong>{event.title}</strong>
                      </button>
                    ))}
                    {items.length > 3 && (
                      <small className="calendar-day__more">+{items.length - 3} more</small>
                    )}
                  </div>
                )
              })}
            </div>
          ))}
        </div>

        <aside className="calendar-agenda" aria-label="Day agenda">
          <div className="calendar-agenda__head">
            <div>
              <p className="eyebrow">Agenda</p>
              <h2>
                {new Date(`${viewDate}T00:00:00`).toLocaleDateString([], {
                  weekday: 'long',
                  month: 'long',
                  day: 'numeric',
                })}
              </h2>
            </div>
            <button
              type="button"
              className="icon-button"
              aria-label="Add event on this day"
              onClick={() => openNew(viewDate)}
            >
              <Plus size={16} />
            </button>
          </div>
          {dayEvents.length === 0 && (
            <p className="settings-hint">Nothing scheduled — create an event for this day.</p>
          )}
          {dayEvents.map((event) => (
            <div className="agenda-row" key={event.id} onClick={() => openEvent(event)}>
              <i className={`agenda-row__dot agenda-row__dot--${event.category}`} />
              <div>
                <strong>{event.title}</strong>
                <small>
                  <Clock3 size={12} />
                  {timeLabel(event)}
                  {event.location ? ` · ${event.location}` : ''}
                </small>
              </div>
              <span>
                {event.invitees.length > 0
                  ? `${event.invitees.filter((invitee) => invitee.status === 'accepted').length}/${event.invitees.length} attending`
                  : 'No invitees'}
              </span>
            </div>
          ))}
          {notice && <p className="settings-notice settings-notice--ok">{notice}</p>}
        </aside>
      </div>

      {editor && (
        <EventEditor
          draft={editor.draft}
          editing={editor.event}
          onClose={() => setEditor(null)}
          onSave={save}
          onDelete={editor.event ? () => remove(editor.event as CalendarEvent) : undefined}
          onExport={editor.event ? () => downloadIcs(editor.event as CalendarEvent) : undefined}
        />
      )}
    </div>
  )
}

type EventEditorProps = {
  draft: Omit<CalendarEvent, 'id'>
  editing: CalendarEvent | null
  onClose: () => void
  onSave: (draft: Omit<CalendarEvent, 'id'>, existing: CalendarEvent | null) => void
  onDelete?: () => void
  onExport?: () => void
}

function EventEditor({
  draft: initial,
  editing,
  onClose,
  onSave,
  onDelete,
  onExport,
}: EventEditorProps) {
  const panelRef = useRef<HTMLElement>(null)
  useFocusTrap(panelRef)
  useEffect(() => {
    const esc = (event: KeyboardEvent) => {
      if (event.key === 'Escape') onClose()
    }
    window.addEventListener('keydown', esc)
    return () => window.removeEventListener('keydown', esc)
  }, [onClose])
  const [draft, setDraft] = useState(initial)
  const [attendees, setAttendees] = useState(() =>
    initial.invitees.map((invitee) => invitee.email).join(', '),
  )
  const update = (patch: Partial<Omit<CalendarEvent, 'id'>>) =>
    setDraft((current) => ({ ...current, ...patch }))

  const submit = (event: FormEvent) => {
    event.preventDefault()
    const invitees = attendees
      .split(',')
      .map((email) => email.trim())
      .filter(Boolean)
      .map((email, _index) => ({
        email,
        status: (editing?.invitees.find(
          (invitee) => invitee.email.toLowerCase() === email.toLowerCase(),
        )?.status ?? 'pending') as 'pending' | 'accepted' | 'declined',
      }))
    onSave({ ...draft, invitees }, editing)
  }

  return (
    <div className="settings-layer" role="presentation">
      <section
        ref={panelRef}
        className="settings-panel settings-panel--narrow"
        role="dialog"
        aria-modal="true"
        aria-label={editing ? 'Edit event' : 'New event'}
      >
        <header>
          <div>
            <p className="eyebrow">Calendar</p>
            <h2>{editing ? 'Edit event' : 'New event'}</h2>
          </div>
          <button
            type="button"
            className="icon-button"
            aria-label="Close event editor"
            onClick={onClose}
          >
            <X size={17} />
          </button>
        </header>
        <form className="billing-body" onSubmit={submit}>
          <div className="settings-section">
            <label>
              Title
              <input
                value={draft.title}
                onChange={(event) => update({ title: event.target.value })}
                placeholder="What's happening?"
                aria-label="Event title"
                autoFocus
              />
            </label>
            <label>
              Date
              <input
                type="date"
                value={draft.date}
                onChange={(event) => update({ date: event.target.value })}
                aria-label="Event date"
              />
            </label>
            <div className="settings-options">
              <label>
                <input
                  type="checkbox"
                  checked={draft.allDay}
                  onChange={(event) => update({ allDay: event.target.checked })}
                />{' '}
                All day event
              </label>
            </div>
            <div className="payment-form__row">
              <label>
                Start
                <input
                  type="time"
                  value={draft.start}
                  disabled={draft.allDay}
                  onChange={(event) => update({ start: event.target.value })}
                  aria-label="Event start time"
                />
              </label>
              <label>
                End
                <input
                  type="time"
                  value={draft.end}
                  disabled={draft.allDay}
                  onChange={(event) => update({ end: event.target.value })}
                  aria-label="Event end time"
                />
              </label>
            </div>
            <label>
              Category
              <select
                value={draft.category}
                onChange={(event) => update({ category: event.target.value as EventCategory })}
                aria-label="Event category"
              >
                {eventCategories.map((category) => (
                  <option key={category.id} value={category.id}>
                    {category.label}
                  </option>
                ))}
              </select>
            </label>
            <label>
              Location
              <input
                value={draft.location}
                onChange={(event) => update({ location: event.target.value })}
                placeholder="Room, link or place"
                aria-label="Event location"
              />
            </label>
            <label>
              Attendees
              <input
                value={attendees}
                onChange={(event) => setAttendees(event.target.value)}
                placeholder="nora@harbor.co, priya@harbor.co"
                aria-label="Event attendees"
              />
            </label>
            <label>
              Description
              <textarea
                value={draft.description}
                onChange={(event) => update({ description: event.target.value })}
                placeholder="Add any details..."
                aria-label="Event description"
              />
            </label>
          </div>
          {editing && draft.invitees.length > 0 && (
            <div className="settings-section">
              <h3>Responses</h3>
              {draft.invitees.map((invitee) => (
                <div className="billing-row" key={invitee.email}>
                  <div>
                    <strong>{invitee.email}</strong>
                  </div>
                  <span className={invitee.status === 'accepted' ? 'billing-paid' : ''}>
                    {invitee.status === 'accepted' && '✓ '}
                    {invitee.status === 'pending'
                      ? 'Invited'
                      : invitee.status === 'accepted'
                        ? 'Accepted'
                        : 'Declined'}
                  </span>
                </div>
              ))}
            </div>
          )}
          <footer className="event-editor-footer">
            <div className="calendar-editor__left">
              {onExport && (
                <button type="button" className="secondary-button" onClick={onExport}>
                  <Download size={14} />
                  Export .ics
                </button>
              )}
              {onDelete && (
                <button
                  type="button"
                  className="secondary-button calendar-editor__delete"
                  onClick={onDelete}
                >
                  <Trash2 size={14} />
                  Delete
                </button>
              )}
            </div>
            <div className="row-actions">
              <button type="button" className="secondary-button" onClick={onClose}>
                Cancel
              </button>
              <button type="submit" className="primary-button">
                <CalendarDays size={15} />
                {editing ? 'Save changes' : 'Create event'}
              </button>
            </div>
          </footer>
        </form>
      </section>
    </div>
  )
}
