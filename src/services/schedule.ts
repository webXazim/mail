import type { Draft } from '../types'

export type ScheduledMessage = { id: string; draft: Draft; at: string }

const scheduleKey = 'harbor-mail:scheduled'

export const scheduleApi = {
  list(): ScheduledMessage[] {
    try {
      return JSON.parse(localStorage.getItem(scheduleKey) || '[]') as ScheduledMessage[]
    } catch {
      return []
    }
  },
  enqueue(message: ScheduledMessage) {
    localStorage.setItem(scheduleKey, JSON.stringify([...this.list(), message]))
    return message
  },
  remove(id: string) {
    localStorage.setItem(
      scheduleKey,
      JSON.stringify(this.list().filter((message) => message.id !== id)),
    )
  },
  dueItems(now = Date.now()): ScheduledMessage[] {
    return this.list().filter((message) => new Date(message.at).getTime() <= now)
  },
}
