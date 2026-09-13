import { messages } from '../data'
import type { Mail } from '../types'
import { primaryAccountId } from './accounts'

const mailboxKey = 'harbor-mail:mailbox'
const draftKey = 'harbor-mail:compose-draft'
const mailboxKeyFor = (accountId: string) =>
  accountId === primaryAccountId ? mailboxKey : `harbor-mail:mailbox:${accountId}`
const delay = (value: Mail[] | null) =>
  new Promise<Mail[]>((resolve) => window.setTimeout(() => resolve(value || messages), 120))
const delayList = (value: Mail[] | null) =>
  new Promise<Mail[]>((resolve) => window.setTimeout(() => resolve(value ?? []), 120))

export const mailboxApi = {
  async list(): Promise<Mail[]> {
    try {
      return delay(JSON.parse(localStorage.getItem(mailboxKey) || 'null') as Mail[] | null)
    } catch {
      return delay(messages)
    }
  },
  async replace(next: Mail[]) {
    localStorage.setItem(mailboxKey, JSON.stringify(next))
    return next
  },
  async listFor(accountId: string): Promise<Mail[]> {
    if (accountId === primaryAccountId) return this.list()
    try {
      const raw = localStorage.getItem(mailboxKeyFor(accountId))
      return raw ? delayList(JSON.parse(raw) as Mail[]) : delayList([])
    } catch {
      return delayList([])
    }
  },
  async replaceFor(accountId: string, next: Mail[]) {
    localStorage.setItem(mailboxKeyFor(accountId), JSON.stringify(next))
    return next
  },
  async saveDraft<T extends object>(draft: T) {
    localStorage.setItem(draftKey, JSON.stringify(draft))
    return draft
  },
  async clearDraft() {
    localStorage.removeItem(draftKey)
  },
}