import type { Draft } from '../types'
import { authApi } from './auth'

export const draftKey = 'cs-mail:compose-draft'

export const draftsApi = {
  load(): Draft | null {
    // Authenticated production drafts are server-authoritative. This cache is
    // only the offline/demo compose scratchpad.
    if (!authApi.isDemo()) return null
    try {
      const saved = JSON.parse(localStorage.getItem(draftKey) || '{}') as Partial<Draft>
      return saved.to ||
        saved.subject ||
        saved.body ||
        (Array.isArray(saved.attachments) && saved.attachments.length > 0)
        ? (saved as Draft)
        : null
    } catch {
      return null
    }
  },
  clear() {
    if (authApi.isDemo()) localStorage.removeItem(draftKey)
  },
}
