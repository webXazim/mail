import { useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { ArrowLeft, Plus, Trash2 } from 'lucide-react'
import { labelColors, labelsApi, type ManagedLabel } from '../services/labels'

export function LabelsPage() {
  const navigate = useNavigate()
  const [list, setList] = useState<ManagedLabel[]>(() => labelsApi.list())
  const [name, setName] = useState('')
  const [color, setColor] = useState<string>(labelColors[0])

  const add = () => {
    setList(labelsApi.add(name, color))
    setName('')
  }
  const remove = (id: string) => setList(labelsApi.remove(id))

  return (
    <div className="settings-page" role="region" aria-label="Labels">
      <header className="calendar-head">
        <div>
          <p className="eyebrow">CS Mail</p>
          <h1>Labels</h1>
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

      <p className="settings-hint">Labels are a browser-local organization preference on this deployment; they do not change server-side mail routing.</p>

      <div className="settings-section">
        <h2>Your labels</h2>
        {list.map((label) => (
          <div className="labels-row" key={label.id}>
            <span className={`label-dot label-dot--${label.color}`} />
            <input
              value={label.name}
              aria-label={`Rename ${label.name}`}
              onChange={(event) => {
                setList(labelsApi.rename(label.id, event.target.value))
              }}
            />
            <select
              aria-label="Label color"
              value={label.color}
              onChange={(event) => {
                setList(labelsApi.recolor(label.id, event.target.value))
              }}
            >
              {labelColors.map((item) => (
                <option key={item} value={item}>
                  {item}
                </option>
              ))}
            </select>
            <button
              type="button"
              className="icon-button"
              aria-label={`Delete ${label.name}`}
              onClick={() => remove(label.id)}
            >
              <Trash2 size={15} />
            </button>
          </div>
        ))}
        {list.length === 0 && <p className="settings-hint">No labels yet — add one below.</p>}
      </div>

      <div className="settings-section">
        <h2>Add a label</h2>
        <div className="labels-row labels-row--add">
          <input
            value={name}
            placeholder="Label name"
            aria-label="New label name"
            onChange={(event) => setName(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === 'Enter') {
                event.preventDefault()
                add()
              }
            }}
          />
          <select
            aria-label="New label color"
            value={color}
            onChange={(event) => setColor(event.target.value)}
          >
            {labelColors.map((item) => (
              <option key={item} value={item}>
                {item}
              </option>
            ))}
          </select>
          <button type="button" className="primary-button" onClick={add}>
            <Plus size={15} />
            Add
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
