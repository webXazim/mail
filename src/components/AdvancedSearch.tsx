import { useState } from 'react'

const scopeOptions = [
  { value: '', label: 'All except Trash & Spam' },
  { value: 'inbox', label: 'Inbox' },
  { value: 'sent', label: 'Sent' },
  { value: 'starred', label: 'Starred' },
  { value: 'unread', label: 'Unread' },
  { value: 'all', label: 'Everything (incl. Trash & Spam)' },
  { value: 'trash', label: 'Trash' },
  { value: 'spam', label: 'Spam' },
]

type Fields = {
  from: string
  to: string
  subject: string
  words: string
  attachment: boolean
  unread: boolean
  starred: boolean
  scope: string
}

const emptyFields: Fields = {
  from: '',
  to: '',
  subject: '',
  words: '',
  attachment: false,
  unread: false,
  starred: false,
  scope: '',
}

export function AdvancedSearch({
  onSubmit,
  onClose,
}: {
  onSubmit: (query: string) => void
  onClose: () => void
}) {
  const [fields, setFields] = useState<Fields>(emptyFields)
  const set = <K extends keyof Fields>(key: K, value: Fields[K]) =>
    setFields((current) => ({ ...current, [key]: value }))

  const submit = () => {
    const tokens: string[] = []
    if (fields.scope) tokens.push(`in:${fields.scope}`)
    if (fields.from.trim()) tokens.push(`from:${fields.from.trim()}`)
    if (fields.to.trim()) tokens.push(`to:${fields.to.trim()}`)
    if (fields.subject.trim()) tokens.push(`subject:${fields.subject.trim()}`)
    if (fields.words.trim()) tokens.push(fields.words.trim())
    if (fields.attachment) tokens.push('has:attachment')
    if (fields.unread) tokens.push('is:unread')
    if (fields.starred) tokens.push('is:starred')
    onSubmit(tokens.join(' '))
  }

  const cancel = () => {
    setFields(emptyFields)
    onClose()
  }

  return (
    <div className="search-advanced">
      <div className="search-advanced__field">
        <span className="sr-only">From</span>
        <input
          type="text"
          placeholder="From"
          value={fields.from}
          onChange={(event) => set('from', event.target.value)}
        />
      </div>
      <div className="search-advanced__field">
        <span className="sr-only">To</span>
        <input
          type="text"
          placeholder="To"
          value={fields.to}
          onChange={(event) => set('to', event.target.value)}
        />
      </div>
      <div className="search-advanced__field">
        <span className="sr-only">Subject</span>
        <input
          type="text"
          placeholder="Subject"
          value={fields.subject}
          onChange={(event) => set('subject', event.target.value)}
        />
      </div>
      <div className="search-advanced__field">
        <span className="sr-only">Has the words</span>
        <input
          type="text"
          placeholder="Has the words"
          value={fields.words}
          onChange={(event) => set('words', event.target.value)}
        />
      </div>
      <div className="search-advanced__check">
        <label>
          <input
            type="checkbox"
            checked={fields.attachment}
            onChange={(event) => set('attachment', event.target.checked)}
          />
          Has attachment
        </label>
        <label>
          <input
            type="checkbox"
            checked={fields.unread}
            onChange={(event) => set('unread', event.target.checked)}
          />
          Is unread
        </label>
        <label>
          <input
            type="checkbox"
            checked={fields.starred}
            onChange={(event) => set('starred', event.target.checked)}
          />
          Is starred
        </label>
      </div>
      <div className="search-advanced__field">
        <span className="sr-only">Scope</span>
        <select value={fields.scope} onChange={(event) => set('scope', event.target.value)}>
          {scopeOptions.map((o) => (
            <option key={o.value} value={o.value}>
              {o.label}
            </option>
          ))}
        </select>
      </div>
      <div className="search-advanced__actions">
        <button type="button" onClick={cancel}>
          Cancel
        </button>
        <button type="button" className="search-advanced__submit" onClick={submit}>
          Search
        </button>
      </div>
    </div>
  )
}
