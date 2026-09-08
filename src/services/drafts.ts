import type { Draft } from '../types'

export const draftKey = 'harbor-mail:compose-draft'

export const draftsApi = {
  load(): Draft | null {
    try {
      const saved = JSON.parse(localStorage.getItem(draftKey) || '{}') as Partial<Draft>
      return saved.to || saved.subject || saved.body || (Array.isArray(saved.attachments) && saved.attachments.length > 0) ? saved as Draft : null
    } catch {
      return null
    }
  },
  clear() {
    localStorage.removeItem(draftKey)
  },
}