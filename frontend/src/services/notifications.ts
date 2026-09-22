import { apiFetch } from '../lib/api'
import { isRemoteMail } from './remote-mail'

export type NotificationKind = 'security' | 'scheduled' | 'mail' | 'billing' | 'support' | 'account'

export type NotificationItem = {
  id: string
  icon: NotificationKind
  title: string
  detail: string
  actionUrl: string
  unread: boolean
  createdAt: string
}

type NotificationRow = {
  id: string
  kind: NotificationKind
  title: string
  detail: string
  action_url: string
  read_at: string | null
  created_at: string
}

type NotificationPage = {
  notifications: NotificationRow[]
  unread: number
  has_more: boolean
  next_before: string | null
}

const KEY = 'cs-mail:notifications'

const seed: NotificationItem[] = [
  {
    id: 'demo-security',
    icon: 'security',
    title: 'New sign-in detected',
    detail: 'Demo browser session',
    actionUrl: '/mail/audit-log',
    unread: true,
    createdAt: new Date().toISOString(),
  },
]

const demoLoad = (): NotificationItem[] => {
  try {
    const raw = localStorage.getItem(KEY)
    if (raw) return JSON.parse(raw) as NotificationItem[]
  } catch {
    /* demo cache can be unavailable */
  }
  return seed
}

const demoSave = (items: NotificationItem[]) => {
  try {
    localStorage.setItem(KEY, JSON.stringify(items))
  } catch {
    /* demo cache can be unavailable */
  }
}

const rowToItem = (row: NotificationRow): NotificationItem => ({
  id: row.id,
  icon: row.kind,
  title: row.title,
  detail: row.detail,
  actionUrl: row.action_url,
  unread: !row.read_at,
  createdAt: row.created_at,
})

export const notificationsApi = {
  load(): NotificationItem[] {
    return isRemoteMail() ? [] : demoLoad()
  },

  async list(before?: string | null): Promise<{ items: NotificationItem[]; unread: number; hasMore: boolean; nextBefore: string | null }> {
    if (!isRemoteMail()) {
      const items = demoLoad()
      return { items, unread: items.filter((item) => item.unread).length, hasMore: false, nextBefore: null }
    }
    const query = new URLSearchParams({ limit: '50' })
    if (before) query.set('before', before)
    const page = await apiFetch<NotificationPage>(`/api/notifications?${query}`)
    return {
      items: (page.notifications ?? []).map(rowToItem),
      unread: page.unread ?? 0,
      hasMore: Boolean(page.has_more),
      nextBefore: page.next_before ?? null,
    }
  },

  async markRead(id: string): Promise<void> {
    if (!isRemoteMail()) {
      demoSave(demoLoad().map((item) => (item.id === id ? { ...item, unread: false } : item)))
      return
    }
    await apiFetch(`/api/notifications/${encodeURIComponent(id)}/read`, { method: 'POST' })
  },

  async markAllRead(): Promise<void> {
    if (!isRemoteMail()) {
      demoSave(demoLoad().map((item) => ({ ...item, unread: false })))
      return
    }
    await apiFetch('/api/notifications/read-all', { method: 'POST' })
  },

  async dismiss(id: string): Promise<void> {
    if (!isRemoteMail()) {
      demoSave(demoLoad().filter((item) => item.id !== id))
      return
    }
    await apiFetch(`/api/notifications/${encodeURIComponent(id)}`, { method: 'DELETE' })
  },

  /** Demo-only helper. Production new-mail/security notifications are created by the backend. */
  add(item: Pick<NotificationItem, 'icon' | 'title' | 'detail'>) {
    if (isRemoteMail()) return
    demoSave([
      {
        ...item,
        id: `demo-${Date.now()}`,
        actionUrl: '',
        unread: true,
        createdAt: new Date().toISOString(),
      },
      ...demoLoad(),
    ])
  },
}
