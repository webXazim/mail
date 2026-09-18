export type ManagedFolder = { id: string; name: string }

const foldersKey = 'harbor-mail:folders'

const defaults: ManagedFolder[] = [{ id: 'folder-projects', name: 'Projects' }]

const persist = (list: ManagedFolder[]) => localStorage.setItem(foldersKey, JSON.stringify(list))

export const foldersApi = {
  list(): ManagedFolder[] {
    try {
      const raw = localStorage.getItem(foldersKey)
      if (!raw) return defaults
      const parsed = JSON.parse(raw) as ManagedFolder[]
      return Array.isArray(parsed) ? parsed : defaults
    } catch {
      return defaults
    }
  },
  add(name: string) {
    const trimmed = name.trim()
    if (!trimmed) return this.list()
    const next = [...this.list(), { id: `folder-${Date.now()}`, name: trimmed }]
    persist(next)
    return next
  },
  rename(id: string, name: string) {
    const trimmed = name.trim()
    if (!trimmed) return this.list()
    const next = this.list().map((folder) =>
      folder.id === id ? { ...folder, name: trimmed } : folder,
    )
    persist(next)
    return next
  },
  remove(id: string) {
    const next = this.list().filter((folder) => folder.id !== id)
    persist(next)
    return next
  },
  byId(id: string) {
    return this.list().find((folder) => folder.id === id)
  },
  byName(name: string) {
    return this.list().find((folder) => folder.name.toLowerCase() === name.toLowerCase())
  },
}
