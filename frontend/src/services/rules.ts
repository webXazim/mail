import { apiFetch } from '../lib/api'
import { isRemoteMail } from './remote-mail'
import type { AutomationSync } from './forwarding'

export type RuleCondition =
  | { field: 'from'; value: string }
  | { field: 'to'; value: string }
  | { field: 'subject'; value: string }
  | { field: 'hasAttachment' }
  | { field: 'size'; op: 'larger' | 'smaller'; size: number }
  | {
      field: 'date'
      op: 'before' | 'after' | 'on' | 'not-on' | 'on-or-before' | 'on-or-after'
      value: string
    }

export type RuleAction =
  | { kind: 'label'; value: string }
  | { kind: 'move'; value: string }
  | { kind: 'archive' }
  | { kind: 'keep-in-inbox' }
  | { kind: 'mark-read' }
  | { kind: 'mark-starred' }
  | { kind: 'discard' }
  | { kind: 'forward'; value: string }

export type FilterRule = {
  id: string
  name: string
  enabled: boolean
  position?: number
  conditions: RuleCondition[]
  actions: RuleAction[]
}

type RulesResponse = { rules: FilterRule[]; sync: AutomationSync }
type RuleResponse = { rule: FilterRule; sync: AutomationSync }

const rulesKey = 'cs-mail:rules'
const defaults: FilterRule[] = []
let remoteCache: FilterRule[] = []
let syncCache: AutomationSync | null = null

const localList = (): FilterRule[] => {
  try {
    const raw = localStorage.getItem(rulesKey)
    if (!raw) return defaults
    const parsed = JSON.parse(raw) as FilterRule[]
    return Array.isArray(parsed) ? parsed : defaults
  } catch {
    return defaults
  }
}

const localSave = (rules: FilterRule[]) => {
  localStorage.setItem(rulesKey, JSON.stringify(rules))
  return rules
}

export const rulesApi = {
  list(): FilterRule[] {
    return isRemoteMail() ? [...remoteCache] : localList()
  },
  sync(): AutomationSync | null {
    return syncCache
  },
  async refresh(): Promise<FilterRule[]> {
    if (!isRemoteMail()) return localList()
    const result = await apiFetch<RulesResponse>('/api/mail/rules')
    remoteCache = result.rules
    syncCache = result.sync
    return [...remoteCache]
  },
  async add(rule: FilterRule): Promise<FilterRule[]> {
    if (!isRemoteMail()) return localSave([rule, ...localList()])
    const result = await apiFetch<RuleResponse>('/api/mail/rules', {
      method: 'POST',
      body: JSON.stringify(rule),
    })
    syncCache = result.sync
    remoteCache = [...remoteCache, result.rule].sort((a, b) => (a.position ?? 0) - (b.position ?? 0))
    return [...remoteCache]
  },
  async update(rule: FilterRule): Promise<FilterRule[]> {
    if (!isRemoteMail()) return localSave(localList().map((item) => (item.id === rule.id ? rule : item)))
    const result = await apiFetch<RuleResponse>(`/api/mail/rules/${encodeURIComponent(rule.id)}`, {
      method: 'PUT', body: JSON.stringify(rule),
    })
    syncCache = result.sync
    remoteCache = remoteCache.map((item) => (item.id === rule.id ? result.rule : item))
    return [...remoteCache]
  },
  async remove(id: string): Promise<FilterRule[]> {
    if (!isRemoteMail()) return localSave(localList().filter((rule) => rule.id !== id))
    const result = await apiFetch<{ ok: boolean; sync: AutomationSync }>(`/api/mail/rules/${encodeURIComponent(id)}`, { method: 'DELETE' })
    syncCache = result.sync
    remoteCache = remoteCache.filter((rule) => rule.id !== id).map((rule, position) => ({ ...rule, position }))
    return [...remoteCache]
  },
  async toggle(id: string, enabled: boolean): Promise<FilterRule[]> {
    const current = isRemoteMail() ? remoteCache : localList()
    const rule = current.find((item) => item.id === id)
    if (!rule) return [...current]
    return rulesApi.update({ ...rule, enabled })
  },
  async reorder(ids: string[]): Promise<FilterRule[]> {
    if (!isRemoteMail()) {
      const byId = new Map(localList().map((rule) => [rule.id, rule]))
      return localSave(ids.flatMap((id, position) => {
        const rule = byId.get(id)
        return rule ? [{ ...rule, position }] : []
      }))
    }
    const result = await apiFetch<{ ok: boolean; sync: AutomationSync }>('/api/mail/rules/reorder', {
      method: 'PUT', body: JSON.stringify({ ids }),
    })
    syncCache = result.sync
    const byId = new Map(remoteCache.map((rule) => [rule.id, rule]))
    remoteCache = ids.flatMap((id, position) => {
      const rule = byId.get(id)
      return rule ? [{ ...rule, position }] : []
    })
    return [...remoteCache]
  },
}
