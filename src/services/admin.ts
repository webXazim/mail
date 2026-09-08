export type MailboxStatus = 'active' | 'quarantine' | 'disabled'

export type MailboxAccount = {
  id: string
  email: string
  displayName: string
  status: MailboxStatus
  storageUsedGB: number
}

export type Alias = { id: string; address: string; forwardTo: string }

export type Forwarder = { id: string; from: string; to: string; enabled: boolean }

export type DnsStatus = { mx: boolean; spf: boolean; dkim: boolean; dmarc: boolean }

export type DomainSettings = { domain: string; catchAllEnabled: boolean; catchAll: string }

const mailboxesKey = 'harbor-mail:admin:mailboxes'
const aliasesKey = 'harbor-mail:admin:aliases'
const forwardersKey = 'harbor-mail:admin:forwarders'
const dnsKey = 'harbor-mail:admin:dns'
const domainKey = 'harbor-mail:admin:domain'

export const seedMailboxes: MailboxAccount[] = [
  { id: 'mb-alex', email: 'alex@harbor.co', displayName: 'Alex Morgan', status: 'active', storageUsedGB: 0.4 },
  { id: 'mb-nora', email: 'nora@harbor.co', displayName: 'Nora Harbor', status: 'active', storageUsedGB: 0.2 },
  { id: 'mb-jonas', email: 'jonas@harbor.co', displayName: 'Jonas Meier', status: 'active', storageUsedGB: 0.1 },
  { id: 'mb-dev', email: 'dev@harbor.co', displayName: 'Dev Team', status: 'active', storageUsedGB: 0.1 },
]

export const seedAliases: Alias[] = [
  { id: 'al-support', address: 'support@harbor.co', forwardTo: 'alex@harbor.co' },
  { id: 'al-sales', address: 'sales@harbor.co', forwardTo: 'nora@harbor.co' },
  { id: 'al-hello', address: 'hello@harbor.co', forwardTo: 'alex@harbor.co' },
]

export const seedForwarders: Forwarder[] = [
  { id: 'fw-alex', from: 'alex@harbor.co', to: 'alexmorgan@example.com', enabled: true },
]

export const seedDomain: DomainSettings = { domain: 'harbor.co', catchAllEnabled: false, catchAll: '' }

export const dnsRecords: { id: keyof DnsStatus; name: string; value: string }[] = [
  { id: 'mx', name: 'MX', value: 'harbor.co. 300 IN MX 10 mail.harbor.co.' },
  { id: 'spf', name: 'SPF', value: 'harbor.co. TXT "v=spf1 include:harbor.co ~all"' },
  { id: 'dkim', name: 'DKIM', value: 'harbor-mail._domainkey.harbor.co. TXT "v=DKIM1; k=rsa; p=MIGfMA0GCSqGSIb3DQEBAQUAA4GNADCBiQKBgQC…"' },
  { id: 'dmarc', name: 'DMARC', value: '_dmarc.harbor.co. TXT "v=DMARC1; p=quarantine; rua=mailto:dmarc@harbor.co"' },
]

const read = <T,>(key: string, seed: T): T => {
  try {
    const raw = localStorage.getItem(key)
    if (!raw) return seed
    const parsed = JSON.parse(raw) as T
    return parsed ?? seed
  } catch {
    return seed
  }
}

const readArray = <T,>(key: string, seed: T[]): T[] => {
  try {
    const raw = localStorage.getItem(key)
    if (!raw) return clone(seed)
    const parsed = JSON.parse(raw) as T[]
    return Array.isArray(parsed) ? clone(parsed) : clone(seed)
  } catch {
    return clone(seed)
  }
}

const write = (key: string, value: unknown) => localStorage.setItem(key, JSON.stringify(value))

const clone = <T,>(value: T): T => (Array.isArray(value) ? (value.map(item => ({ ...item })) as T) : { ...value })

export const adminApi = {
  listMailboxes(): MailboxAccount[] {
    return readArray<MailboxAccount>(mailboxesKey, seedMailboxes)
  },
  saveMailboxes(next: MailboxAccount[]) {
    write(mailboxesKey, next)
  },
  addMailbox(input: { email: string; displayName?: string }) {
    const current = this.listMailboxes()
    const raw = input.email.trim().toLowerCase()
    const email = raw.includes('@') ? raw : `${raw}@harbor.co`
    if (!/^[a-z0-9.+-]+@[a-z0-9.-]+\.[a-z]{2,}$/.test(email) || current.some(mailbox => mailbox.email.toLowerCase() === email)) return current
    const next = [
      ...current,
      { id: `mb-${Date.now()}`, email, displayName: input.displayName?.trim() || email.split('@')[0], status: 'active' as const, storageUsedGB: 0 },
    ]
    this.saveMailboxes(next)
    return next
  },
  removeMailbox(id: string) {
    const current = this.listMailboxes()
    const target = current.find(mailbox => mailbox.id === id)
    if (!target || target.email.startsWith('alex@')) return current
    const next = current.filter(mailbox => mailbox.id !== id)
    this.saveMailboxes(next)
    return next
  },

  listAliases(): Alias[] {
    return readArray<Alias>(aliasesKey, seedAliases)
  },
  saveAliases(next: Alias[]) {
    write(aliasesKey, next)
  },
  addAlias(localPart: string, forwardTo: string, domain: string) {
    const current = this.listAliases()
    const address = `${localPart.trim().toLowerCase().split('@')[0]}@${domain}`
    if (current.some(alias => alias.address.toLowerCase() === address)) return current
    const next = [...current, { id: `al-${Date.now()}`, address, forwardTo }]
    this.saveAliases(next)
    return next
  },
  removeAlias(id: string) {
    const next = this.listAliases().filter(alias => alias.id !== id)
    this.saveAliases(next)
    return next
  },

  listForwarders(): Forwarder[] {
    return readArray<Forwarder>(forwardersKey, seedForwarders)
  },
  saveForwarders(next: Forwarder[]) {
    write(forwardersKey, next)
  },
  addForwarder(from: string, to: string) {
    const current = this.listForwarders()
    const trimmed = to.trim()
    if (!trimmed.includes('@') || current.some(forwarder => forwarder.from === from && forwarder.to === trimmed)) return current
    const next = [...current, { id: `fw-${Date.now()}`, from, to: trimmed, enabled: true }]
    this.saveForwarders(next)
    return next
  },
  toggleForwarder(id: string) {
    const next = this.listForwarders().map(forwarder => (forwarder.id === id ? { ...forwarder, enabled: !forwarder.enabled } : forwarder))
    this.saveForwarders(next)
    return next
  },
  removeForwarder(id: string) {
    const next = this.listForwarders().filter(forwarder => forwarder.id !== id)
    this.saveForwarders(next)
    return next
  },

  getDns(): DnsStatus {
    return read<DnsStatus>(dnsKey, { mx: false, spf: false, dkim: false, dmarc: false })
  },
  saveDns(next: DnsStatus) {
    write(dnsKey, next)
  },
  verifyAll() {
    const next: DnsStatus = { mx: true, spf: true, dkim: true, dmarc: true }
    this.saveDns(next)
    return next
  },
  resetDns() {
    const next: DnsStatus = { mx: false, spf: false, dkim: false, dmarc: false }
    this.saveDns(next)
    return next
  },

  getDomainSettings(): DomainSettings {
    return read<DomainSettings>(domainKey, seedDomain)
  },
  saveDomainSettings(next: DomainSettings) {
    write(domainKey, next)
  },
}