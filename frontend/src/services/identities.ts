import { apiFetch } from '../lib/api'
import { primaryAccount } from './accounts'
import { isRemoteMail } from './remote-mail'

export type Identity = {
  id: string
  email: string
  displayName: string
  replyTo?: string
  primary: boolean
  status?: 'pending' | 'verified' | 'disabled' | string
  source?: 'primary' | 'alias' | 'verified_external' | string
  verifiedAt?: string
  verificationExpiresAt?: string
}

type RemoteIdentity = {
  id: string
  email: string
  display_name: string
  reply_to?: string | null
  primary: boolean
  status: string
  source: string
  verified_at?: string | null
  verification_expires_at?: string | null
}

type CreateIdentityResponse = {
  identity: RemoteIdentity
  verificationCode?: string | null
}

type ResendResponse = { ok: boolean; verificationCode?: string | null }

const identitiesKey = 'cs-mail:identities'

export const defaultIdentities: Identity[] = [
  { id: 'id-alex', email: 'alex@crescentsphere.com', displayName: 'Alex Morgan', primary: true },
  { id: 'id-dev', email: 'alex@cs-mail.dev', displayName: 'Alex Morgan', primary: false },
  { id: 'id-nora', email: 'nora@crescentsphere.com', displayName: 'Nora Saleh', primary: false },
]

let remoteCache: Identity[] = []

const seededPrimary = (): Identity => {
  const primary = primaryAccount()
  return { id: 'primary-pending', email: primary.email, displayName: primary.name, primary: true }
}

const mapRemote = (identity: RemoteIdentity): Identity => ({
  id: identity.id,
  email: identity.email,
  displayName: identity.display_name,
  ...(identity.reply_to ? { replyTo: identity.reply_to } : {}),
  primary: identity.primary,
  status: identity.status,
  source: identity.source,
  ...(identity.verified_at ? { verifiedAt: identity.verified_at } : {}),
  ...(identity.verification_expires_at
    ? { verificationExpiresAt: identity.verification_expires_at }
    : {}),
})

const replaceRemote = (identity: RemoteIdentity) => {
  const mapped = mapRemote(identity)
  remoteCache = remoteCache.some((item) => item.id === mapped.id)
    ? remoteCache.map((item) => (item.id === mapped.id ? mapped : item))
    : [...remoteCache, mapped]
  return mapped
}

const localList = (): Identity[] => {
  try {
    const raw = localStorage.getItem(identitiesKey)
    const parsed = raw ? (JSON.parse(raw) as Identity[]) : []
    if (Array.isArray(parsed) && parsed.length) return parsed
  } catch {
    /* fall through to defaults */
  }
  return defaultIdentities
}

export const identitiesApi = {
  list(): Identity[] {
    if (isRemoteMail()) return remoteCache.length ? remoteCache : [seededPrimary()]
    return localList()
  },

  async refresh(): Promise<Identity[]> {
    if (!isRemoteMail()) return this.list()
    const result = await apiFetch<{ identities: RemoteIdentity[] }>('/api/identities')
    remoteCache = (result.identities ?? []).map(mapRemote)
    return remoteCache
  },

  async create(identity: {
    email: string
    displayName: string
    replyTo?: string | null
  }): Promise<{ identities: Identity[]; verificationCode?: string }> {
    if (!isRemoteMail()) {
      return { identities: this.add(identity) }
    }
    const result = await apiFetch<CreateIdentityResponse>('/api/identities', {
      method: 'POST',
      body: JSON.stringify({
        email: identity.email,
        display_name: identity.displayName,
        reply_to: identity.replyTo || null,
      }),
    })
    replaceRemote(result.identity)
    return {
      identities: remoteCache,
      ...(result.verificationCode ? { verificationCode: result.verificationCode } : {}),
    }
  },

  async verify(id: string, code: string): Promise<Identity[]> {
    if (!isRemoteMail()) return this.list()
    const updated = await apiFetch<RemoteIdentity>(
      `/api/identities/${encodeURIComponent(id)}/verify`,
      { method: 'POST', body: JSON.stringify({ code }) },
    )
    replaceRemote(updated)
    return remoteCache
  },

  async resend(id: string): Promise<string | undefined> {
    if (!isRemoteMail()) return undefined
    const result = await apiFetch<ResendResponse>(
      `/api/identities/${encodeURIComponent(id)}/resend`,
      { method: 'POST' },
    )
    await this.refresh()
    return result.verificationCode || undefined
  },

  async update(
    id: string,
    patch: { displayName?: string; replyTo?: string | null },
  ): Promise<Identity[]> {
    if (!isRemoteMail()) {
      if (patch.displayName !== undefined) return this.rename(id, patch.displayName)
      return this.list()
    }
    const body: Record<string, unknown> = {}
    if (patch.displayName !== undefined) body.display_name = patch.displayName
    if ('replyTo' in patch) body.reply_to = patch.replyTo ?? null
    const updated = await apiFetch<RemoteIdentity>(`/api/identities/${encodeURIComponent(id)}`, {
      method: 'PUT',
      body: JSON.stringify(body),
    })
    replaceRemote(updated)
    return remoteCache
  },

  async makeDefault(id: string): Promise<Identity[]> {
    if (!isRemoteMail()) return this.setPrimary(id)
    const updated = await apiFetch<RemoteIdentity>(
      `/api/identities/${encodeURIComponent(id)}/default`,
      { method: 'PUT' },
    )
    const mapped = mapRemote(updated)
    remoteCache = remoteCache.map((item) => ({ ...item, primary: item.id === mapped.id }))
    replaceRemote(updated)
    return remoteCache
  },

  async removeRemote(id: string): Promise<Identity[]> {
    if (!isRemoteMail()) return this.remove(id)
    await apiFetch(`/api/identities/${encodeURIComponent(id)}`, { method: 'DELETE' })
    return this.refresh()
  },

  save(next: Identity[]) {
    if (isRemoteMail()) return
    localStorage.setItem(identitiesKey, JSON.stringify(next))
  },

  add(identity: { email: string; displayName: string }) {
    if (isRemoteMail()) return this.list()
    const email = identity.email.trim().toLowerCase()
    if (!email.includes('@') || this.list().some((item) => item.email.toLowerCase() === email))
      return this.list()
    const next = [
      ...this.list(),
      {
        id: `identity-${Date.now()}`,
        email,
        displayName: identity.displayName.trim() || identity.email.split('@')[0],
        primary: false,
      },
    ]
    this.save(next)
    return next
  },

  rename(id: string, displayName: string) {
    if (isRemoteMail()) return this.list()
    const next = this.list().map((identity) =>
      identity.id === id ? { ...identity, displayName } : identity,
    )
    this.save(next)
    return next
  },

  setPrimary(id: string) {
    if (isRemoteMail()) return this.list()
    const next = this.list().map((identity) => ({ ...identity, primary: identity.id === id }))
    this.save(next)
    return next
  },

  remove(id: string) {
    if (isRemoteMail()) return this.list()
    const current = this.list()
    const target = current.find((identity) => identity.id === id)
    if (!target || target.primary || current.length <= 1) return current
    const next = current.filter((identity) => identity.id !== id)
    this.save(next)
    return next
  },
}
