import { useEffect, useRef, useState } from 'react'
import { Plus, Trash2, X } from 'lucide-react'
import { useFocusTrap } from '../hooks/useFocusTrap'
import { labelColors, labelsApi, type ManagedLabel } from '../services/labels'

export function ManageLabels({ close }: { close: () => void }) {
  const [list, setList] = useState<ManagedLabel[]>(() => labelsApi.list())
  const [name, setName] = useState('')
  const [color, setColor] = useState<string>(labelColors[0])
  const panelRef = useRef<HTMLElement>(null)
  useFocusTrap(panelRef)
  useEffect(() => {
    const esc = (event: KeyboardEvent) => { if (event.key === 'Escape') close() }
    window.addEventListener('keydown', esc)
    return () => window.removeEventListener('keydown', esc)
  }, [close])

  const sync = (next: ManagedLabel[]) => setList(next)

  const add = () => {
    sync(labelsApi.add(name, color))
    setName('')
  }
  const remove = (id: string) => sync(labelsApi.remove(id))

  return (
    <div className="settings-layer" role="presentation">
      <section ref={panelRef} className="settings-panel" role="dialog" aria-modal="true" aria-labelledby="labels-title">
        <header>
          <div><p className="eyebrow">Harbor Mail</p><h2 id="labels-title">Manage labels</h2></div>
          <button type="button" className="icon-button" aria-label="Close labels" onClick={close}><X size={17} /></button>
        </header>
        <div className="labels-body">
          <div className="settings-section">
            <h3>Your labels</h3>
            {list.map(label => (
              <div className="labels-row" key={label.id}>
                <span className={`label-dot label-dot--${label.color}`} />
                <input value={label.name} aria-label={`Rename ${label.name}`} onChange={event => { sync(labelsApi.rename(label.id, event.target.value)) }} />
                <select aria-label="Label color" value={label.color} onChange={event => { sync(labelsApi.recolor(label.id, event.target.value)) }}>
                  {labelColors.map(item => <option key={item} value={item}>{item}</option>)}
                </select>
                <button type="button" className="icon-button" aria-label={`Delete ${label.name}`} onClick={() => remove(label.id)}><Trash2 size={15} /></button>
              </div>
            ))}
            {list.length === 0 && <p className="settings-hint">No labels yet — add one below.</p>}
          </div>
          <div className="settings-section">
            <h3>Add a label</h3>
            <div className="labels-row labels-row--add">
              <input value={name} placeholder="Label name" aria-label="New label name" onChange={event => setName(event.target.value)} onKeyDown={event => { if (event.key === 'Enter') { event.preventDefault(); add() } }} />
              <select aria-label="New label color" value={color} onChange={event => setColor(event.target.value)}>
                {labelColors.map(item => <option key={item} value={item}>{item}</option>)}
              </select>
              <button type="button" className="primary-button" onClick={add}><Plus size={15} />Add</button>
            </div>
          </div>
          <footer>
            <button type="button" className="primary-button" onClick={close}>Done</button>
          </footer>
        </div>
      </section>
    </div>
  )
}