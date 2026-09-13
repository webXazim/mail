export type AccountAuditCategory = 'sign-in' | 'security' | 'billing' | 'general'

export type AccountAuditEntry = {
  id: string
  time: string
  category: AccountAuditCategory
  action: string
  detail: string
}

const key = 'harbor-mail:account-audit'

const seed: AccountAuditEntry[] = [
  {
    id: 'aud-1',
    time: '2026-09-11T14:32:00.000Z',
    category: 'sign-in',
    action: 'Signed in',
    detail: 'Chrome on macOS',
  },
  {
    id: 'aud-2',
    time: '2026-09-10T09:05:00.000Z',
    category: 'security',
    action: 'Email verified',
    detail: 'You confirmed alex@harbor.co',
  },
  {
    id: 'aud-3',
    time: '2026-09-04T18:20:00.000Z',
    category: 'billing',
    action: 'Payment method updated',
    detail: 'Added Visa ending in 4049',
  },
  {
    id: 'aud-4',
    time: '2026-08-28T11:47:00.000Z',
    category: 'security',
    action: 'Password changed',
    detail: 'Changed from the security settings',
  },
  {
    id: 'aud-5',
    time: '2026-08-01T02:00:00.000Z',
    category: 'billing',
    action: 'Plan charged',
    detail: 'INV-2026-033 · $8.00',
  },
]

export const auditApi = {
  list(): AccountAuditEntry[] {
    try {
      const raw = localStorage.getItem(key)
      if (raw) return JSON.parse(raw) as AccountAuditEntry[]
    } catch {
      /* ignore corrupt cache */
    }
    return seed
  },
  add(category: AccountAuditCategory, action: string, detail: string) {
    const entry: AccountAuditEntry = {
      id: `aud-${Date.now()}`,
      time: new Date().toISOString(),
      category,
      action,
      detail,
    }
    localStorage.setItem(key, JSON.stringify([entry, ...this.list()].slice(0, 50)))
  },
  clear() {
    localStorage.setItem(key, JSON.stringify([]))
  },
}
