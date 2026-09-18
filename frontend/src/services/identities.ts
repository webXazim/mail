import { primaryAccount } from './accounts'
import { isRemoteMail } from './remote-mail'

export type Identity = { id: string; email: string; displayName: string; primary: boolean }

const identitiesKey = 'harbor-mail:identities'

export const defaultIdentities: Identity[] = [
  { id: 'id-alex', email: 'alex@harbor.co', displayName: 'Alex Morgan', primary: true },
  { id: 'id-dev', email: 'alex@harbor.dev', displayName: 'Alex Morgan', primary: false },
  { id: 'id-nora', email: 'nora@harbor.co', displayName: 'Nora Harbor', primary: false },
]

const seededPrimary = (): Identity => {
  const primary = primaryAccount()
  return { id: 'id-primary', email: primary.email, displayName: primary.name, primary: true }
}

export const identitiesApi = {
  list(): Identity[] {
    try {
      const raw = localStorage.getItem(identitiesKey)
      const parsed = raw ? (JSON.parse(raw) as Identity[]) : []
      if (Array.isArray(parsed) && parsed.length) return parsed
    } catch {
      /* fall through to defaults */
    }
    // Real users get their own identity, never the demo cast.
    return isRemoteMail() ? [seededPrimary()] : defaultIdentities
  },
  save(next: Identity[]) {
    localStorage.setItem(identitiesKey, JSON.stringify(next))
  },
  add(identity: { email: string; displayName: string }) {
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
    const next = this.list().map((identity) =>
      identity.id === id ? { ...identity, displayName } : identity,
    )
    this.save(next)
    return next
  },
  setPrimary(id: string) {
    const next = this.list().map((identity) => ({ ...identity, primary: identity.id === id }))
    this.save(next)
    return next
  },
  remove(id: string) {
    const current = this.list()
    const target = current.find((identity) => identity.id === id)
    if (!target || target.primary || current.length <= 1) return current
    const next = current.filter((identity) => identity.id !== id)
    this.save(next)
    return next
  },
}
