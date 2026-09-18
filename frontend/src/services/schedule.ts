import type { Draft } from '../types'
import { apiFetch } from '../lib/api'
import { getAttachmentPayload } from './attachments'
import { composeToDraft, isRemoteMail, parseRecipients, type RemoteCompose } from './remote-mail'

export type ScheduledMessage = { id: string; draft: Draft; at: string }

type ScheduledRow = { id: string; send_at: string; compose: RemoteCompose }

const scheduleKey = 'harbor-mail:scheduled'

/** Server rows use UUID ids; locally-queued (offline/demo) rows use `scheduled-`. */
const isServerId = (id: string) => !id.startsWith('scheduled-')

const readCache = (): ScheduledMessage[] => {
  try {
    const raw = JSON.parse(localStorage.getItem(scheduleKey) || '[]')
    return Array.isArray(raw) ? (raw as ScheduledMessage[]) : []
  } catch {
    return []
  }
}

const writeCache = (items: ScheduledMessage[]) => {
  try {
    localStorage.setItem(scheduleKey, JSON.stringify(items))
  } catch {
    /* quota exceeded — the in-memory list still renders this session */
  }
}

const draftToCompose = (draft: Draft): RemoteCompose => ({
  to: parseRecipients(draft.to),
  cc: parseRecipients(draft.cc),
  bcc: parseRecipients(draft.bcc),
  subject: draft.subject,
  body_text: draft.body,
  attachments: draft.attachments
    .map(getAttachmentPayload)
    .filter((payload): payload is NonNullable<typeof payload> => payload !== null),
})

const rowToMessage = (row: ScheduledRow): ScheduledMessage => ({
  id: row.id,
  at: row.send_at,
  draft: { ...composeToDraft(row.compose), scheduledAt: row.send_at },
})

export const scheduleApi = {
  /** Synchronous read from the local cache. */
  list(): ScheduledMessage[] {
    return readCache()
  },
  /**
   * API-first refresh. Server rows replace previous server rows, while
   * locally-queued sends (offline fallback) are preserved for the client to
   * deliver so nothing is lost when the API call failed.
   */
  async refresh(): Promise<ScheduledMessage[]> {
    if (!isRemoteMail()) return this.list()
    try {
      const result = await apiFetch<{ scheduled: ScheduledRow[] }>('/api/scheduled')
      const local = this.list().filter((message) => !isServerId(message.id))
      const next = [...local, ...(result.scheduled ?? []).map(rowToMessage)]
      writeCache(next)
      return next
    } catch {
      return this.list()
    }
  },
  async enqueue(message: ScheduledMessage): Promise<ScheduledMessage> {
    if (isRemoteMail()) {
      try {
        const row = await apiFetch<ScheduledRow>('/api/scheduled', {
          method: 'POST',
          body: JSON.stringify({ send_at: message.at, ...draftToCompose(message.draft) }),
        })
        const next = [
          ...this.list().filter((existing) => existing.id !== message.id && existing.id !== row.id),
          rowToMessage(row),
        ]
        writeCache(next)
        return next[next.length - 1]
      } catch {
        // Offline: keep the optimistic local queue so the client can send it.
      }
    }
    writeCache([...this.list(), message])
    return message
  },
  async remove(id: string): Promise<void> {
    if (isRemoteMail() && isServerId(id)) {
      try {
        await apiFetch(`/api/scheduled/${encodeURIComponent(id)}`, { method: 'DELETE' })
      } catch {
        // Fall through to the local removal so the UI stays responsive offline.
      }
    }
    writeCache(this.list().filter((message) => message.id !== id))
  },
  /**
   * Only client-queued sends fire here. With a live session the backend worker
   * delivers server rows, so returning [] for them avoids double-sending.
   */
  dueItems(now = Date.now()): ScheduledMessage[] {
    return this.list().filter(
      (message) => !isServerId(message.id) && new Date(message.at).getTime() <= now,
    )
  },
}
