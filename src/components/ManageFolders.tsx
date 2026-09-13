import { useEffect, useRef, useState } from 'react'
import { Folder, Plus, Trash2, X } from 'lucide-react'
import { useFocusTrap } from '../hooks/useFocusTrap'
import { foldersApi, type ManagedFolder } from '../services/folders'

export function ManageFolders({ close }: { close: () => void }) {
  const [list, setList] = useState<ManagedFolder[]>(() => foldersApi.list())
  const [name, setName] = useState('')
  const panelRef = useRef<HTMLElement>(null)
  useFocusTrap(panelRef)
  useEffect(() => {
    const esc = (event: KeyboardEvent) => {
      if (event.key === 'Escape') close()
    }
    window.addEventListener('keydown', esc)
    return () => window.removeEventListener('keydown', esc)
  }, [close])

  const sync = (next: ManagedFolder[]) => setList(next)

  const add = () => {
    sync(foldersApi.add(name))
    setName('')
  }
  const remove = (id: string) => sync(foldersApi.remove(id))

  return (
    <div className="settings-layer" role="presentation">
      <section
        ref={panelRef}
        className="settings-panel"
        role="dialog"
        aria-modal="true"
        aria-labelledby="folders-title"
      >
        <header>
          <div>
            <p className="eyebrow">Harbor Mail</p>
            <h2 id="folders-title">Manage folders</h2>
          </div>
          <button type="button" className="icon-button" aria-label="Close folders" onClick={close}>
            <X size={17} />
          </button>
        </header>
        <div className="labels-body">
          <div className="settings-section">
            <h3>Your folders</h3>
            {list.map((customFolder) => (
              <div className="folders-row" key={customFolder.id}>
                <Folder size={15} />
                <input
                  value={customFolder.name}
                  aria-label={`Rename ${customFolder.name}`}
                  onChange={(event) => {
                    sync(foldersApi.rename(customFolder.id, event.target.value))
                  }}
                />
                <button
                  type="button"
                  className="icon-button"
                  aria-label={`Delete ${customFolder.name}`}
                  onClick={() => remove(customFolder.id)}
                >
                  <Trash2 size={15} />
                </button>
              </div>
            ))}
            {list.length === 0 && (
              <p className="settings-hint">No custom folders yet — add one below.</p>
            )}
            <p className="settings-hint">
              Messages can be moved into custom folders from the list toolbar, the reader, or the
              move menu.
            </p>
          </div>
          <div className="settings-section">
            <h3>Add a folder</h3>
            <div className="folders-row folders-row--add">
              <input
                value={name}
                placeholder="Folder name"
                aria-label="New folder name"
                onChange={(event) => setName(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === 'Enter') {
                    event.preventDefault()
                    add()
                  }
                }}
              />
              <button type="button" className="primary-button" onClick={add}>
                <Plus size={15} />
                Add
              </button>
            </div>
          </div>
          <footer>
            <button type="button" className="primary-button" onClick={close}>
              Done
            </button>
          </footer>
        </div>
      </section>
    </div>
  )
}
