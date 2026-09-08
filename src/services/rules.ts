export type RuleCondition =
  | { field: 'from'; value: string }
  | { field: 'to'; value: string }
  | { field: 'subject'; value: string }
  | { field: 'hasAttachment' }
  | { field: 'size'; op: 'larger' | 'smaller'; size: number }
  | { field: 'date'; op: 'before' | 'after' | 'on' | 'not-on' | 'on-or-before' | 'on-or-after'; value: string }

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
  conditions: RuleCondition[]
  actions: RuleAction[]
}

const rulesKey = 'harbor-mail:rules'

const defaults: FilterRule[] = [
  {
    id: 'rule-newsletters',
    name: 'Newsletters',
    enabled: true,
    conditions: [{ field: 'from', value: 'news@harbor.co' }],
    actions: [{ kind: 'label', value: 'Internal' }],
  },
]

export const rulesApi = {
  list(): FilterRule[] {
    try {
      const raw = localStorage.getItem(rulesKey)
      if (!raw) return defaults
      const parsed = JSON.parse(raw) as FilterRule[]
      return Array.isArray(parsed) && parsed.length ? parsed : defaults
    } catch {
      return defaults
    }
  },
  save(rules: FilterRule[]) {
    localStorage.setItem(rulesKey, JSON.stringify(rules))
    return rules
  },
  add(rule: FilterRule) {
    return this.save([rule, ...this.list()])
  },
  update(rule: FilterRule) {
    return this.save(this.list().map(item => (item.id === rule.id ? rule : item)))
  },
  remove(id: string) {
    return this.save(this.list().filter(rule => rule.id !== id))
  },
  toggle(id: string, enabled: boolean) {
    return this.save(this.list().map(rule => (rule.id === id ? { ...rule, enabled } : rule)))
  },
}