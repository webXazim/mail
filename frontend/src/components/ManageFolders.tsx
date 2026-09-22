import { useEffect, useRef, useState } from 'react'
import { Folder, Plus, Trash2, X } from 'lucide-react'
import { useFocusTrap } from '../hooks/useFocusTrap'
import { foldersApi, type ManagedFolder } from '../services/folders'

export function ManageFolders({ close }: { close: () => void }) {
  const [list, setList] = useState<ManagedFolder[]>(() => foldersApi.list())
  const [name, setName] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
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

  const run = async (work: () => Promise<ManagedFolder[]>) => {
    setBusy(true)
    setError('')
    try {
      sync(await work())
      return true
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : 'Folder update failed')
      return false
    } finally {
      setBusy(false)
    }
  }
  const add = async () => {
    const ok = await run(() => foldersApi.add(name))
    if (ok) setName('')
  }
  const remove = (id: string) => void run(() => foldersApi.remove(id))

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
            <p className="eyebrow">CS Mail</p>
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
                    const nextName = event.target.value
                    setList((current) =>
                      current.map((folder) =>
                        folder.id === customFolder.id ? { ...folder, name: nextName } : folder,
                      ),
                    )
                  }}
                  onBlur={(event) => void run(() => foldersApi.rename(customFolder.id, event.target.value))}
                />
                <button
                  type="button"
                  className="icon-button"
                  aria-label={`Delete ${customFolder.name}`}
                  disabled={busy}
                  onClick={() => remove(customFolder.id)}
                >
                  <Trash2 size={15} />
                </button>
              </div>
            ))}
            {error && <p className="settings-hint" role="alert">{error}</p>}
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
                    void add()
                  }
                }}
              />
              <button type="button" className="primary-button" disabled={busy} onClick={() => void add()}>
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
