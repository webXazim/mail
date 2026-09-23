import { useEffect, useState } from 'react'
import { mailboxContextStore, tokenStore } from '../lib/api'

export type ResourceChangedPayload = {
  resource: string
  action: string
  id?: string
  version?: number
  folders_changed?: boolean
  mailbox_id?: string
  organization_id?: string
}

export type RealtimeEvent =
  | {
      seq: number
      kind: 'quota'
      payload: { used: number; total: number; provider_total?: number; quota_in_sync?: boolean; mailbox_id?: string }
      created_at?: string
    }
  | {
      seq: number
      kind: 'new-mail'
      payload: { id: string; thread_id?: string; from: string; subject: string; received_at?: string; mailbox_id?: string }
      created_at?: string
    }
  | {
      seq: number
      kind: 'resource-changed'
      payload: ResourceChangedPayload
      created_at?: string
    }
  | {
      seq: number
      kind: 'ready' | 'resync-required' | 'pong'
      payload: Record<string, unknown>
      created_at?: string
    }

export type RealtimeKind = RealtimeEvent['kind'] | '*'

const cursorKey = () => `cs-mail:realtime-cursor:${mailboxContextStore.getMailboxId() ?? 'global'}`
const heartbeatMs = 25_000
const maxBackoffMs = 30_000

type PollResponse = {
  events: RealtimeEvent[]
  cursor: number
  resync_required: boolean
}

const wsBaseUrl = () =>
  (import.meta.env.VITE_WS_URL as string | undefined) ??
  `${location.protocol === 'https:' ? 'wss' : 'ws'}://${location.host}/api/ws`

const isLiveSession = () => {
  const token = tokenStore.getAccess()
  return Boolean(token && !token.startsWith('demo.'))
}

const readCursor = (): number => {
  try {
    const value = Number.parseInt(sessionStorage.getItem(cursorKey()) ?? '0', 10)
    return Number.isFinite(value) && value >= 0 ? value : 0
  } catch {
    return 0
  }
}

const writeCursor = (value: number) => {
  if (!Number.isFinite(value) || value < 0) return
  try {
    sessionStorage.setItem(cursorKey(), String(Math.trunc(value)))
  } catch {
    /* private storage may be unavailable; reconnect still falls back to a live cursor */
  }
}

const realtimeUrl = (poll = false, after = readCursor()) => {
  const url = new URL(wsBaseUrl(), location.href)
  if (after > 0) url.searchParams.set('after', String(after))
  const organizationId = mailboxContextStore.getOrganizationId()
  const mailboxId = mailboxContextStore.getMailboxId()
  if (organizationId) url.searchParams.set('organization_id', organizationId)
  if (mailboxId) url.searchParams.set('mailbox_id', mailboxId)
  if (poll) {
    url.protocol = location.protocol
    url.searchParams.set('poll', '1')
    url.searchParams.set('wait', '25')
  }
  return url.toString()
}

const dispatchResourceEvent = (event: RealtimeEvent) => {
  window.dispatchEvent(new CustomEvent('cs-mail-realtime', { detail: event }))
  if (event.kind !== 'resource-changed') return

  window.dispatchEvent(new CustomEvent('cs-mail-resource-changed', { detail: event }))
  if (event.payload.resource === 'mailbox' && event.payload.folders_changed) {
    window.dispatchEvent(new Event('cs-mail-folders-changed'))
  }
}

const dispatchFullResync = () => {
  const resources = [
    'mailbox',
    'contacts',
    'calendar',
    'settings',
    'drafts',
    'schedule',
    'automation',
    'identities',
    'profile',
    'notifications',
    'support',
  ]
  for (const resource of resources) {
    const event: RealtimeEvent = {
      seq: readCursor(),
      kind: 'resource-changed',
      payload: { resource, action: 'resync', folders_changed: resource === 'mailbox' },
    }
    dispatchResourceEvent(event)
  }
}

export class CSMailSocket {
  private socket: WebSocket | null = null
  private retry = 0
  private listeners = new Set<(event: RealtimeEvent) => void>()
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null
  private heartbeatTimer: ReturnType<typeof setInterval> | null = null
  private pollAbort: AbortController | null = null
  private started = false

  constructor() {
    if (typeof window === 'undefined') return
    window.addEventListener('cs-mail-auth-changed', this.syncAuth)
    window.addEventListener('cs-mail-mailbox-context-changed', this.handleMailboxContextChange)
    window.addEventListener('online', this.handleOnline)
    window.addEventListener('offline', this.handleOffline)
    window.addEventListener('visibilitychange', this.handleVisibility)
    this.syncAuth()
  }

  private syncAuth = () => {
    if (isLiveSession()) this.start()
    else this.stop()
  }


  private handleMailboxContextChange = () => {
    if (!this.started || !isLiveSession()) return
    this.retry = 0
    this.clearReconnect()
    this.closeTransport()
    dispatchFullResync()
    this.connectNow()
  }

  private handleOnline = () => {
    if (this.started) this.connectNow()
  }

  private handleOffline = () => {
    this.closeTransport()
  }

  private handleVisibility = () => {
    if (document.visibilityState === 'visible' && this.started && !this.isOpen()) {
      this.connectNow()
    }
  }

  start() {
    if (!isLiveSession()) return
    this.started = true
    this.connectNow()
  }

  stop() {
    this.started = false
    this.retry = 0
    this.clearReconnect()
    this.closeTransport()
  }

  private isOpen() {
    return this.socket?.readyState === WebSocket.OPEN
  }

  private connectNow() {
    if (!this.started || !isLiveSession() || !navigator.onLine) return
    if (
      this.socket &&
      (this.socket.readyState === WebSocket.CONNECTING || this.socket.readyState === WebSocket.OPEN)
    ) return
    this.clearReconnect()
    this.pollAbort?.abort()
    this.pollAbort = null

    try {
      const socket = new WebSocket(realtimeUrl(false))
      this.socket = socket
      socket.onopen = () => {
        this.retry = 0
        this.startHeartbeat()
      }
      socket.onmessage = (message) => {
        try {
          const parsed = JSON.parse(String(message.data)) as RealtimeEvent | RealtimeEvent[]
          for (const event of Array.isArray(parsed) ? parsed : [parsed]) this.accept(event)
        } catch {
          /* malformed server frames are ignored; the cursor never advances */
        }
      }
      socket.onclose = () => {
        if (this.socket === socket) this.socket = null
        this.stopHeartbeat()
        if (this.started && isLiveSession()) this.scheduleReconnect()
      }
      socket.onerror = () => socket.close()
    } catch {
      this.scheduleReconnect()
    }
  }

  private accept(event: RealtimeEvent) {
    if (!event || !Number.isFinite(event.seq) || typeof event.kind !== 'string') return

    if (event.kind === 'ready') {
      const resync = Boolean(event.payload?.resync_required)
      if (event.seq >= 0) writeCursor(event.seq)
      if (resync) dispatchFullResync()
      return
    }
    if (event.kind === 'resync-required') {
      if (event.seq >= 0) writeCursor(event.seq)
      dispatchFullResync()
      return
    }
    if (event.kind === 'pong') return

    const cursor = readCursor()
    if (event.seq <= cursor) return
    writeCursor(event.seq)
    dispatchResourceEvent(event)
    for (const listener of this.listeners) listener(event)
  }

  private startHeartbeat() {
    this.stopHeartbeat()
    this.heartbeatTimer = setInterval(() => {
      if (this.socket?.readyState === WebSocket.OPEN) {
        try {
          this.socket.send(JSON.stringify({ type: 'ping' }))
        } catch {
          this.socket.close()
        }
      }
    }, heartbeatMs)
  }

  private stopHeartbeat() {
    if (this.heartbeatTimer) clearInterval(this.heartbeatTimer)
    this.heartbeatTimer = null
  }

  private closeTransport() {
    this.stopHeartbeat()
    this.pollAbort?.abort()
    this.pollAbort = null
    if (this.socket) {
      const socket = this.socket
      this.socket = null
      try {
        socket.close()
      } catch {
        /* already closed */
      }
    }
  }

  private clearReconnect() {
    if (this.reconnectTimer) clearTimeout(this.reconnectTimer)
    this.reconnectTimer = null
  }

  private scheduleReconnect() {
    if (this.reconnectTimer || !this.started || !isLiveSession()) return
    const exponent = Math.min(this.retry++, 5)
    const base = Math.min(maxBackoffMs, 1_000 * 2 ** exponent)
    const jitter = Math.floor(Math.random() * Math.min(1_000, base / 4))
    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = null
      void this.longPollOnce().finally(() => {
        if (this.started && !this.isOpen()) this.connectNow()
      })
    }, base + jitter)
  }

  private async longPollOnce() {
    if (!this.started || !isLiveSession() || !navigator.onLine || this.isOpen()) return
    this.pollAbort?.abort()
    const controller = new AbortController()
    this.pollAbort = controller
    try {
      const response = await fetch(realtimeUrl(true), {
        credentials: 'include',
        signal: controller.signal,
        headers: { Accept: 'application/json' },
      })
      if (response.status === 401) {
        this.stop()
        return
      }
      if (!response.ok) return
      const payload = (await response.json()) as PollResponse
      if (payload.resync_required) dispatchFullResync()
      for (const event of payload.events ?? []) this.accept(event)
      if (Number.isFinite(payload.cursor) && payload.cursor > readCursor()) writeCursor(payload.cursor)
    } catch (error) {
      if (!(error instanceof DOMException && error.name === 'AbortError')) {
        /* websocket retry owns user-facing recovery */
      }
    } finally {
      if (this.pollAbort === controller) this.pollAbort = null
    }
  }

  on(kind: RealtimeKind, callback: (event: RealtimeEvent) => void) {
    const listener = (event: RealtimeEvent) => {
      if (kind === '*' || event.kind === kind) callback(event)
    }
    this.listeners.add(listener)
    return () => { this.listeners.delete(listener) }
  }
}

export const csMailSocket = new CSMailSocket()

export function useMailboxEvents() {
  const [last, setLast] = useState<RealtimeEvent | null>(null)
  useEffect(() => csMailSocket.on('*', setLast), [])
  return last
}
