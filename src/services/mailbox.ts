import { messages } from '../data'
import type { Mail } from '../types'
import { apiBase } from './backend'

const mailboxKey = 'harbor-mail:mailbox'
const draftKey = 'harbor-mail:compose-draft'
const delay = (value: Mail[] | null) => new Promise<Mail[]>(resolve => window.setTimeout(() => resolve(value || messages), 120))

export const mailboxApi = {
  async list(): Promise<Mail[]> {
    if (apiBase) { const response = await fetch(`${apiBase}/mail`, { credentials: 'include' }); if (!response.ok) throw new Error('Unable to load mailbox'); return response.json() as Promise<Mail[]> }
    try { return delay(JSON.parse(localStorage.getItem(mailboxKey) || 'null') as Mail[] | null) } catch { return delay(messages) }
  },
  async replace(next: Mail[]) { if (apiBase) { const response = await fetch(`${apiBase}/mail`, { method: 'PUT', credentials: 'include', headers: { 'content-type': 'application/json' }, body: JSON.stringify(next) }); if (!response.ok) throw new Error('Unable to update mailbox'); return response.json() as Promise<Mail[]> } localStorage.setItem(mailboxKey, JSON.stringify(next)); return next },
  async saveDraft<T extends object>(draft: T) { localStorage.setItem(draftKey, JSON.stringify(draft)); return draft },
  async clearDraft() { localStorage.removeItem(draftKey) },
}
