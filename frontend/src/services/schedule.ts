import type { Draft } from '../types'
import { apiFetch } from '../lib/api'
import { composeToDraft, isRemoteMail, parseRecipients, type RemoteCompose } from './remote-mail'

export type ScheduledStatus = 'pending' | 'processing' | 'retry' | 'dead'
export type ScheduledMessage = {
  id: string
  draft: Draft
  at: string
  status?: ScheduledStatus
  error?: string
  attemptCount?: number
  nextAttemptAt?: string
}

type ScheduledRow = {
  id: string
  send_at: string
  compose: RemoteCompose
  status?: ScheduledStatus
  error?: string | null
  attempt_count?: number
  next_attempt_at?: string | null
}

const scheduleKey = 'cs-mail:scheduled'

/** Server rows use UUID ids; locally-queued (offline/demo) rows use `scheduled-`. */
const isServerId = (id: string) => !id.startsWith('scheduled-')

const readCache = (): ScheduledMessage[] => {
  try {
    const raw = JSON.parse(localStorage.getItem(scheduleKey) || '[]')
    if (!Array.isArray(raw)) return []
    return raw.map((item) => ({
      ...item,
      status: item.status || 'pending',
      attemptCount: Number(item.attemptCount || 0),
    })) as ScheduledMessage[]
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
  attachments: draft.attachments.map((attachment) => ({ id: attachment.id })),
  identity_id: draft.identityId,
  client_key: draft.clientKey,
  send_key: draft.sendKey,
})

const rowToMessage = (row: ScheduledRow): ScheduledMessage => ({
  id: row.id,
  at: row.send_at,
  draft: { ...composeToDraft(row.compose), scheduledAt: row.send_at },
  status: row.status || 'pending',
  error: row.error || undefined,
  attemptCount: row.attempt_count ?? 0,
  nextAttemptAt: row.next_attempt_at || undefined,
})

export const scheduleApi = {
  /** Synchronous read from the local display cache. */
  list(): ScheduledMessage[] {
    return readCache()
  },
  /** Server-authoritative refresh. Demo mode may use the local queue, but an
   * authenticated production session never invents a client-side scheduled send. */
  async refresh(): Promise<ScheduledMessage[]> {
    if (!isRemoteMail()) return this.list()
    const result = await apiFetch<{ scheduled: ScheduledRow[] }>('/api/scheduled')
    const next = (result.scheduled ?? []).map(rowToMessage)
    writeCache(next)
    return next
  },
  async enqueue(message: ScheduledMessage): Promise<ScheduledMessage> {
    if (isRemoteMail()) {
      // A compose version already owns a durable send key. Prefixing that UUID
      // gives schedule creation a stable idempotency key across lost responses,
      // refreshes and multi-device retries without reusing the eventual delivery key.
      const idempotencyKey = `schedule:${message.draft.sendKey || crypto.randomUUID()}`
      const row = await apiFetch<ScheduledRow>('/api/scheduled', {
        method: 'POST',
        headers: { 'Idempotency-Key': idempotencyKey },
        body: JSON.stringify({ send_at: message.at, ...draftToCompose(message.draft) }),
      })
      const next = [
        ...this.list().filter((existing) => existing.id !== message.id && existing.id !== row.id),
        rowToMessage(row),
      ]
      writeCache(next)
      return next[next.length - 1]
    }
    writeCache([...this.list(), { ...message, status: message.status || 'pending', attemptCount: 0 }])
    return message
  },
  async remove(id: string): Promise<void> {
    if (isRemoteMail() && isServerId(id)) {
      await apiFetch(`/api/scheduled/${encodeURIComponent(id)}`, { method: 'DELETE' })
    }
    writeCache(this.list().filter((message) => message.id !== id))
  },
  async retry(id: string): Promise<void> {
    if (!isRemoteMail() || !isServerId(id)) return
    await apiFetch(`/api/scheduled/${encodeURIComponent(id)}/retry`, { method: 'POST' })
  },
  /**
   * Only client-queued sends fire here. With a live session the leased backend
   * worker delivers server rows, so returning [] for them avoids double-sending.
   */
  dueItems(now = Date.now()): ScheduledMessage[] {
    return this.list().filter(
      (message) => !isServerId(message.id) && new Date(message.at).getTime() <= now,
    )
  },
}
