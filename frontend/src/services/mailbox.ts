import { messages } from '../data'
import type { Mail } from '../types'
import { primaryAccountId } from './accounts'

const mailboxKey = 'cs-mail:mailbox'
const mailboxKeyFor = (accountId: string) =>
  accountId === primaryAccountId ? mailboxKey : `cs-mail:mailbox:${accountId}`
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
}
