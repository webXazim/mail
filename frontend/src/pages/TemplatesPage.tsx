import { useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { ArrowLeft, Plus, Trash2 } from 'lucide-react'
import { templatesApi, type Template } from '../services/templates'

export function TemplatesPage() {
  const navigate = useNavigate()
  const [list, setList] = useState<Template[]>(() => templatesApi.list())
  const [name, setName] = useState('')
  const [to, setTo] = useState('')
  const [subject, setSubject] = useState('')
  const [body, setBody] = useState('')

  const add = () => {
    if (!name.trim()) return
    setList(
      templatesApi.add({ name: name.trim(), to: to.trim(), cc: '', subject: subject.trim(), body }),
    )
    setName('')
    setTo('')
    setSubject('')
    setBody('')
  }
  const remove = (id: string) => setList(templatesApi.remove(id))
  const update = (id: string, field: 'name' | 'to' | 'cc' | 'subject' | 'body', value: string) =>
    setList(templatesApi.update(id, { [field]: value }))

  return (
    <div className="settings-page" role="region" aria-label="Templates">
      <header className="calendar-head">
        <div>
          <p className="eyebrow">CS Mail</p>
          <h1>Templates</h1>
        </div>
        <div className="calendar-head__actions">
          <button
            type="button"
            className="secondary-button"
            onClick={() => navigate('/mail/inbox')}
          >
            <ArrowLeft size={14} />
            Back to inbox
          </button>
        </div>
      </header>

      <p className="settings-hint">Templates are stored in this browser on this deployment. They are a compose convenience, not a server-enforced mail setting.</p>

      <div className="settings-section">
        <h2>Your templates</h2>
        {list.map((template) => (
          <div className="template-card" key={template.id}>
            <div className="template-card__row">
              <input
                value={template.name}
                aria-label="Template name"
                placeholder="Name"
                onChange={(event) => update(template.id, 'name', event.target.value)}
              />
              <input
                value={template.to}
                aria-label="Template recipients"
                placeholder="To (optional)"
                onChange={(event) => update(template.id, 'to', event.target.value)}
              />
              <input
                value={template.subject}
                aria-label="Template subject"
                placeholder="Subject"
                onChange={(event) => update(template.id, 'subject', event.target.value)}
              />
              <button
                type="button"
                className="icon-button"
                aria-label={`Delete ${template.name}`}
                onClick={() => remove(template.id)}
              >
                <Trash2 size={15} />
              </button>
            </div>
            <textarea
              className="template-card__body"
              value={template.body}
              aria-label="Template body"
              rows={3}
              placeholder="Message body — use {name} for the recipient's name"
              onChange={(event) => update(template.id, 'body', event.target.value)}
            />
          </div>
        ))}
        {list.length === 0 && <p className="settings-hint">No templates yet — add one below.</p>}
      </div>

      <div className="settings-section">
        <h2>Add a template</h2>
        <div className="template-card template-card--add">
          <div className="template-card__row">
            <input
              value={name}
              placeholder="Name"
              aria-label="New template name"
              onChange={(event) => setName(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === 'Enter') {
                  event.preventDefault()
                  add()
                }
              }}
            />
            <input
              value={to}
              placeholder="To (optional)"
              aria-label="New template recipients"
              onChange={(event) => setTo(event.target.value)}
            />
            <input
              value={subject}
              placeholder="Subject"
              aria-label="New template subject"
              onChange={(event) => setSubject(event.target.value)}
            />
          </div>
          <textarea
            className="template-card__body"
            value={body}
            aria-label="New template body"
            rows={3}
            placeholder="Message body"
            onChange={(event) => setBody(event.target.value)}
          />
          <button type="button" className="primary-button template-card__add" onClick={add}>
            <Plus size={15} />
            Add template
          </button>
        </div>
      </div>

      <footer>
        <button type="button" className="primary-button" onClick={() => navigate('/mail/inbox')}>
          Done
        </button>
      </footer>
    </div>
  )
}
