import { apiFetch } from '../lib/api'
import { authApi } from './auth'

export type ManagedFolder = { id: string; name: string; total?: number; unread?: number }

const foldersKey = 'cs-mail:folders'
const defaults: ManagedFolder[] = [{ id: 'folder-projects', name: 'Projects' }]
let remoteCache: ManagedFolder[] = []

const demoList = (): ManagedFolder[] => {
  try {
    const raw = localStorage.getItem(foldersKey)
    if (!raw) return defaults
    const parsed = JSON.parse(raw) as ManagedFolder[]
    return Array.isArray(parsed) ? parsed : defaults
  } catch {
    return defaults
  }
}

const persistDemo = (list: ManagedFolder[]) => localStorage.setItem(foldersKey, JSON.stringify(list))
const announce = () => window.dispatchEvent(new CustomEvent('cs-mail-folders-changed'))

export const foldersApi = {
  list(): ManagedFolder[] {
    return authApi.isDemo() ? demoList() : remoteCache
  },
  syncRemote(list: ManagedFolder[]) {
    remoteCache = list
  },
  async add(name: string): Promise<ManagedFolder[]> {
    const trimmed = name.trim()
    if (!trimmed) return this.list()
    if (authApi.isDemo()) {
      const next = [...demoList(), { id: `folder-${Date.now()}`, name: trimmed }]
      persistDemo(next)
      announce()
      return next
    }
    const created = await apiFetch<{ id: string; name: string }>('/api/mail/mailboxes', {
      method: 'POST',
      body: JSON.stringify({ name: trimmed }),
    })
    remoteCache = [...remoteCache, { id: created.id, name: created.name }]
    announce()
    return remoteCache
  },
  async rename(id: string, name: string): Promise<ManagedFolder[]> {
    const trimmed = name.trim()
    if (!trimmed) return this.list()
    if (authApi.isDemo()) {
      const next = demoList().map((folder) => (folder.id === id ? { ...folder, name: trimmed } : folder))
      persistDemo(next)
      announce()
      return next
    }
    await apiFetch(`/api/mail/mailboxes/${encodeURIComponent(id)}`, {
      method: 'PUT',
      body: JSON.stringify({ name: trimmed }),
    })
    remoteCache = remoteCache.map((folder) => (folder.id === id ? { ...folder, name: trimmed } : folder))
    announce()
    return remoteCache
  },
  async remove(id: string): Promise<ManagedFolder[]> {
    if (authApi.isDemo()) {
      const next = demoList().filter((folder) => folder.id !== id)
      persistDemo(next)
      announce()
      return next
    }
    await apiFetch(`/api/mail/mailboxes/${encodeURIComponent(id)}`, { method: 'DELETE' })
    remoteCache = remoteCache.filter((folder) => folder.id !== id)
    announce()
    return remoteCache
  },
  byId(id: string) {
    return this.list().find((folder) => folder.id === id)
  },
  byName(name: string) {
    return this.list().find((folder) => folder.name.toLowerCase() === name.toLowerCase())
  },
}
