import { apiFetch } from '../lib/api'
import { isRemoteMail } from './remote-mail'

export type AutomationSync = {
  status: 'pending' | 'syncing' | 'ready' | 'error' | 'disabled'
  desiredRevision: number
  appliedRevision: number
  inSync: boolean
  lastError: string
  appliedAt?: string | null
}

export type ForwardingSettings = {
  enabled: boolean
  address: string
  keepCopy: boolean
  verified?: boolean
  verifiedAt?: string | null
  verificationPending?: boolean
  verificationExpiresAt?: string | null
}

export type ForwardingResponse = {
  forwarding: ForwardingSettings
  sync: AutomationSync
  verificationCode?: string | null
}

export const defaultForwarding: ForwardingSettings = {
  enabled: false,
  address: '',
  keepCopy: true,
  verified: false,
  verifiedAt: null,
  verificationPending: false,
  verificationExpiresAt: null,
}

const forwardingKey = 'cs-mail:forwarding'
let remoteCache: ForwardingSettings = { ...defaultForwarding }
let syncCache: AutomationSync | null = null

const localLoad = (): ForwardingSettings => {
  try {
    return { ...defaultForwarding, ...JSON.parse(localStorage.getItem(forwardingKey) || '{}') }
  } catch {
    return { ...defaultForwarding }
  }
}

export const forwardingApi = {
  load(): ForwardingSettings {
    return isRemoteMail() ? { ...remoteCache } : localLoad()
  },
  sync(): AutomationSync | null {
    return syncCache
  },
  async refresh(): Promise<ForwardingResponse> {
    if (!isRemoteMail()) {
      return {
        forwarding: localLoad(),
        sync: {
          status: 'disabled',
          desiredRevision: 0,
          appliedRevision: 0,
          inSync: true,
          lastError: '',
        },
      }
    }
    const result = await apiFetch<ForwardingResponse>('/api/mail/forwarding')
    remoteCache = { ...defaultForwarding, ...result.forwarding }
    syncCache = result.sync
    return { ...result, forwarding: { ...remoteCache } }
  },
  async save(next: ForwardingSettings): Promise<ForwardingResponse> {
    if (!isRemoteMail()) {
      const local = { ...defaultForwarding, ...next }
      localStorage.setItem(forwardingKey, JSON.stringify(local))
      return {
        forwarding: local,
        sync: {
          status: 'disabled',
          desiredRevision: 0,
          appliedRevision: 0,
          inSync: true,
          lastError: '',
        },
      }
    }
    const result = await apiFetch<ForwardingResponse>('/api/mail/forwarding', {
      method: 'PUT',
      body: JSON.stringify({
        enabled: next.enabled,
        address: next.address,
        keepCopy: next.keepCopy,
      }),
    })
    remoteCache = { ...defaultForwarding, ...result.forwarding }
    syncCache = result.sync
    return { ...result, forwarding: { ...remoteCache } }
  },
  async verify(code: string): Promise<ForwardingResponse> {
    const result = await apiFetch<ForwardingResponse>('/api/mail/forwarding/verify', {
      method: 'POST',
      body: JSON.stringify({ code }),
    })
    remoteCache = { ...defaultForwarding, ...result.forwarding }
    syncCache = result.sync
    return { ...result, forwarding: { ...remoteCache } }
  },
  async resend(): Promise<{ ok: boolean; verificationCode?: string | null }> {
    return apiFetch('/api/mail/forwarding/resend', { method: 'POST' })
  },
}
