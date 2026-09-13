export type Template = {
  id: string
  name: string
  to: string
  cc: string
  subject: string
  body: string
}

const templatesKey = 'harbor-mail:templates'

export const seedTemplates: Template[] = [
  {
    id: 'tpl-followup',
    name: 'Follow up',
    to: '',
    cc: '',
    subject: 'Following up',
    body: 'Hi {name},\n\nI wanted to check in on my last message and see if you had a moment to discuss next steps.\n\nBest,\nAlex',
  },
  {
    id: 'tpl-agenda',
    name: 'Meeting agenda',
    to: '',
    cc: '',
    subject: 'Agenda for our call',
    body: 'Hi {name},\n\nHere is the agenda for our call:\n\n1. Recap of open items\n2. Decisions needed\n3. Timeline and owners\n\nDoes anything need to be added?\n\nThanks,\nAlex',
  },
]

export const templatesApi = {
  list(): Template[] {
    try {
      const stored = JSON.parse(localStorage.getItem(templatesKey) || 'null') as Template[] | null
      return stored ?? seedTemplates
    } catch {
      return seedTemplates
    }
  },
  save(next: Template[]) {
    localStorage.setItem(templatesKey, JSON.stringify(next))
  },
  add(input: Omit<Template, 'id'>): Template[] {
    const next = [{ id: `tpl-${Date.now()}`, ...input }, ...this.list()]
    this.save(next)
    return next
  },
  update(id: string, patch: Partial<Omit<Template, 'id'>>): Template[] {
    const next = this.list().map((template) =>
      template.id === id ? { ...template, ...patch } : template,
    )
    this.save(next)
    return next
  },
  remove(id: string): Template[] {
    const next = this.list().filter((template) => template.id !== id)
    this.save(next)
    return next
  },
}
