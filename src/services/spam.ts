export type SpamLevel = 'low' | 'medium' | 'aggressive'

export type SpamSettings = { blocked: string[]; allowed: string[]; spamLevel: SpamLevel }

export const defaultSpam: SpamSettings = { blocked: [], allowed: [], spamLevel: 'medium' }

const spamKey = 'harbor-mail:spam'

const normalize = (address: string) => address.trim().toLowerCase()

export const spamApi = {
  load(): SpamSettings {
    try {
      return { ...defaultSpam, ...JSON.parse(localStorage.getItem(spamKey) || '{}') }
    } catch {
      return defaultSpam
    }
  },
  save(next: SpamSettings) {
    localStorage.setItem(spamKey, JSON.stringify(next))
    return next
  },
  block(address: string) {
    const target = normalize(address)
    if (!target) return this.load()
    const next = { ...this.load(), blocked: [...this.load().blocked, target], allowed: this.load().allowed.filter(item => item !== target) }
    return this.save(next)
  },
  allow(address: string) {
    const target = normalize(address)
    if (!target) return this.load()
    const next = { ...this.load(), allowed: [...this.load().allowed, target], blocked: this.load().blocked.filter(item => item !== target) }
    return this.save(next)
  },
  removeBlocked(address: string) {
    const next = { ...this.load(), blocked: this.load().blocked.filter(item => item !== normalize(address)) }
    return this.save(next)
  },
  removeAllowed(address: string) {
    const next = { ...this.load(), allowed: this.load().allowed.filter(item => item !== normalize(address)) }
    return this.save(next)
  },
}