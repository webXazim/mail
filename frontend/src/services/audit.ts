import { apiFetch } from '../lib/api'
import { isRemoteMail } from './remote-mail'

export type AccountAuditCategory = 'sign-in' | 'security' | 'billing' | 'mail' | 'general'

export type AccountAuditEntry = {
  id: string
  time: string
  category: AccountAuditCategory
  action: string
  detail: string
}

type ActivityPage = {
  entries: AccountAuditEntry[]
  has_more: boolean
  next_before: string | null
}

const key = 'cs-mail:account-audit'

const demoEntries: AccountAuditEntry[] = [
  {
    id: 'demo-audit-1',
    time: new Date().toISOString(),
    category: 'sign-in',
    action: 'Signed in',
    detail: 'Demo browser session',
  },
]

const demoList = (): AccountAuditEntry[] => {
  try {
    const raw = localStorage.getItem(key)
    if (raw) return JSON.parse(raw) as AccountAuditEntry[]
  } catch {
    /* ignore demo cache errors */
  }
  return demoEntries
}

export const auditApi = {
  list(): AccountAuditEntry[] {
    return isRemoteMail() ? [] : demoList()
  },

  async page(category: 'all' | AccountAuditCategory = 'all', before?: string | null) {
    if (!isRemoteMail()) {
      const entries = demoList().filter((entry) => category === 'all' || entry.category === category)
      return { entries, hasMore: false, nextBefore: null }
    }
    const query = new URLSearchParams({ limit: '50' })
    if (category !== 'all') query.set('category', category)
    if (before) query.set('before', before)
    const page = await apiFetch<ActivityPage>(`/api/account/activity?${query}`)
    return {
      entries: page.entries ?? [],
      hasMore: Boolean(page.has_more),
      nextBefore: page.next_before ?? null,
    }
  },

  /** Demo-only projection; authenticated activity is already recorded by the server. */
  add(category: AccountAuditCategory, action: string, detail: string) {
    if (isRemoteMail()) return
    const entry: AccountAuditEntry = {
      id: `demo-aud-${Date.now()}`,
      time: new Date().toISOString(),
      category,
      action,
      detail,
    }
    try {
      localStorage.setItem(key, JSON.stringify([entry, ...demoList()].slice(0, 50)))
    } catch {
      /* ignore demo cache errors */
    }
  },
}
