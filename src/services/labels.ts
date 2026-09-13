export type ManagedLabel = { id: string; name: string; color: string }

export const labelColors = ['coral', 'green', 'blue', 'amber', 'purple'] as const

const labelsKey = 'harbor-mail:labels'

const defaults: ManagedLabel[] = [
  { id: 'l-clients', name: 'Clients', color: 'coral' },
  { id: 'l-finance', name: 'Finance', color: 'green' },
  { id: 'l-internal', name: 'Internal', color: 'blue' },
  { id: 'l-important', name: 'Important', color: 'amber' },
  { id: 'l-attachment', name: 'Attachment', color: 'purple' },
]

const persist = (list: ManagedLabel[]) => localStorage.setItem(labelsKey, JSON.stringify(list))

export const labelsApi = {
  list(): ManagedLabel[] {
    try {
      const raw = localStorage.getItem(labelsKey)
      if (!raw) return defaults
      const parsed = JSON.parse(raw) as ManagedLabel[]
      return Array.isArray(parsed) && parsed.length ? parsed : defaults
    } catch {
      return defaults
    }
  },
  add(name: string, color: string) {
    const trimmed = name.trim()
    if (!trimmed) return this.list()
    const next = [...this.list(), { id: `label-${Date.now()}`, name: trimmed, color }]
    persist(next)
    return next
  },
  rename(id: string, name: string) {
    const trimmed = name.trim()
    if (!trimmed) return this.list()
    const next = this.list().map((label) => (label.id === id ? { ...label, name: trimmed } : label))
    persist(next)
    return next
  },
  recolor(id: string, color: string) {
    const next = this.list().map((label) => (label.id === id ? { ...label, color } : label))
    persist(next)
    return next
  },
  remove(id: string) {
    const next = this.list().filter((label) => label.id !== id)
    persist(next)
    return next.length ? next : defaults
  },
}
