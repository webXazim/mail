import { useEffect, useState } from 'react'

export type MailboxEvent =
  | { kind: 'quota'; payload: { used: number; total: number } }
  | { kind: 'new-mail'; payload: { id: string; from: string; subject: string } }
  | { kind: 'profile'; payload: { name: string; role: string } }

const wsUrl = () =>
  (import.meta.env.VITE_WS_URL as string | undefined) ??
  `${location.protocol === 'https:' ? 'wss' : 'ws'}://${location.host}/api/ws`

export class HarborSocket {
  private socket: WebSocket | null = null
  private retry = 0
  private queue: MailboxEvent[] = []
  private listeners = new Set<(event: MailboxEvent) => void>()
  private timer: ReturnType<typeof setTimeout> | null = null

  constructor() {
    this.connect()
  }

  private connect() {
    try {
      const ws = new WebSocket(wsUrl())
      this.socket = ws
      ws.onopen = () => {
        this.retry = 0
        this.drain()
      }
      ws.onmessage = (message) => {
        try {
          const events = JSON.parse(message.data) as MailboxEvent | MailboxEvent[]
          ;(Array.isArray(events) ? events : [events]).forEach((event) => this.push(event))
        } catch {
          /* drop malformed frames */
        }
      }
      ws.onclose = () => this.scheduleReconnect()
      ws.onerror = () => ws.close()
    } catch {
      this.scheduleReconnect()
    }
  }

  private scheduleReconnect() {
    if (this.timer) return
    const backoff = Math.min(30_000, 1_000 * 2 ** this.retry++)
    this.timer = setTimeout(() => {
      this.timer = null
      this.longPoll()
      if (this.socket?.readyState !== WebSocket.OPEN) this.connect()
    }, backoff)
  }

  private async longPoll() {
    try {
      const response = await fetch('/api/ws?poll=1')
      if (!response.ok) return
      const events = (await response.json()) as MailboxEvent[]
      events.forEach((event) => this.push(event))
    } finally {
      this.scheduleReconnect()
    }
  }

  private push(event: MailboxEvent) {
    this.queue.push(event)
    const callbacks = this.listeners
    if (this.socket?.readyState === WebSocket.OPEN) {
      this.drain()
      callbacks.forEach((callback) => callback(event))
    }
  }

  private drain() {
    while (this.queue.length && this.socket?.readyState === WebSocket.OPEN) {
      const event = this.queue.shift()
      if (event) this.listeners.forEach((callback) => callback(event))
    }
  }

  on(event: MailboxEvent | 'quota' | 'new-mail' | 'profile', callback: (e: MailboxEvent) => void) {
    this.listeners.add((incoming) => {
      if (event === 'quota' && incoming.kind === 'quota') return callback(incoming)
      if (event === 'new-mail' && incoming.kind === 'new-mail') return callback(incoming)
      if (event === 'profile' && incoming.kind === 'profile') return callback(incoming)
    })
    return () => this.listeners.clear()
  }
}

export const harborSocket = new HarborSocket()

export function useMailboxEvents() {
  const [last, setLast] = useState<MailboxEvent | null>(null)
  useEffect(() => harborSocket.on('quota', (event) => setLast(event)), [])
  return last
}
