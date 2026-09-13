import { mailboxApi } from './mailbox'
import { primaryAccountId } from './accounts'
import type { Mail } from '../types'

export type MailboxStatus = 'active' | 'quarantine' | 'disabled'

export type Role = 'owner' | 'admin' | 'member'

export type MailboxAccount = {
  id: string
  email: string
  displayName: string
  status: MailboxStatus
  role: Role
  storageUsedGB: number
  quotaGB: number
}

export type Alias = { id: string; address: string; forwardTo: string }

export type Forwarder = { id: string; from: string; to: string; enabled: boolean }

export type DnsStatus = { mx: boolean; spf: boolean; dkim: boolean; dmarc: boolean }

export type DomainSettings = { domain: string; catchAllEnabled: boolean; catchAll: string }

export type SecuritySettings = {
  spamThreshold: number
  requireTls: boolean
  scanAttachments: boolean
  retentionDays: number
  dmarcPolicy: 'none' | 'quarantine' | 'reject'
  blockedSenders: string[]
  trashAutoPurge: boolean
}

export type SsoSettings = {
  enabled: boolean
  provider: 'okta' | 'entra' | 'google' | 'custom'
  entityId: string
  enforce: boolean
}

export const ssoProviders: Record<SsoSettings['provider'], string> = {
  okta: 'Okta',
  entra: 'Microsoft Entra ID',
  google: 'Google Workspace',
  custom: 'Custom SAML 2.0',
}

export type QuarantinedMail = {
  id: string
  from: string
  to: string
  subject: string
  date: string
  reason: string
  sizeKB: number
}

export type AuditEntry = {
  id: string
  time: string
  actor: string
  action: string
  detail: string
}

const mailboxesKey = 'harbor-mail:admin:mailboxes'
const aliasesKey = 'harbor-mail:admin:aliases'
const forwardersKey = 'harbor-mail:admin:forwarders'
const dnsKey = 'harbor-mail:admin:dns'
const domainKey = 'harbor-mail:admin:domain'
const securityKey = 'harbor-mail:admin:security'
const quarantineKey = 'harbor-mail:admin:quarantine'
const auditKey = 'harbor-mail:admin:audit'
const passwordsKey = 'harbor-mail:admin:passwords'
const ssoKey = 'harbor-mail:admin:sso'

export const seedMailboxes: MailboxAccount[] = [
  {
    id: 'mb-alex',
    email: 'alex@harbor.co',
    displayName: 'Alex Morgan',
    status: 'active',
    role: 'owner',
    storageUsedGB: 0.4,
    quotaGB: 10,
  },
  {
    id: 'mb-nora',
    email: 'nora@harbor.co',
    displayName: 'Nora Harbor',
    status: 'active',
    role: 'member',
    storageUsedGB: 0.2,
    quotaGB: 10,
  },
  {
    id: 'mb-jonas',
    email: 'jonas@harbor.co',
    displayName: 'Jonas Meier',
    status: 'active',
    role: 'member',
    storageUsedGB: 0.1,
    quotaGB: 10,
  },
  {
    id: 'mb-dev',
    email: 'dev@harbor.co',
    displayName: 'Dev Team',
    status: 'active',
    role: 'member',
    storageUsedGB: 0.1,
    quotaGB: 5,
  },
]

export const seedAliases: Alias[] = [
  { id: 'al-support', address: 'support@harbor.co', forwardTo: 'alex@harbor.co' },
  { id: 'al-sales', address: 'sales@harbor.co', forwardTo: 'nora@harbor.co' },
  { id: 'al-hello', address: 'hello@harbor.co', forwardTo: 'alex@harbor.co' },
]

export const seedForwarders: Forwarder[] = [
  { id: 'fw-alex', from: 'alex@harbor.co', to: 'alexmorgan@example.com', enabled: true },
]

export const seedDomain: DomainSettings = {
  domain: 'harbor.co',
  catchAllEnabled: false,
  catchAll: '',
}

export const seedSecurity: SecuritySettings = {
  spamThreshold: 5,
  requireTls: false,
  scanAttachments: true,
  retentionDays: 0,
  dmarcPolicy: 'quarantine',
  blockedSenders: ['prize@win-now.example'],
  trashAutoPurge: false,
}

export const seedSso: SsoSettings = {
  enabled: false,
  provider: 'okta',
  entityId: 'https://harbor.co/saml2',
  enforce: false,
}

const atTime = (minutesAgo: number): string =>
  new Date(Date.now() - minutesAgo * 60_000).toISOString()

export const seedQuarantine: QuarantinedMail[] = [
  {
    id: 'q-1',
    from: 'prize@win-now.example',
    to: 'alex@harbor.co',
    subject: 'You have won a prize!',
    date: atTime(35),
    reason: 'Blocked by global blocklist',
    sizeKB: 4,
  },
  {
    id: 'q-2',
    from: 'billing@ghost-invoice.example',
    to: 'nora@harbor.co',
    subject: 'Invoice overdue — pay immediately',
    date: atTime(120),
    reason: 'High spam score (8.4 / 10)',
    sizeKB: 18,
  },
]

export const seedAudit: AuditEntry[] = [
  {
    id: 'au-1',
    time: atTime(130),
    actor: 'admin',
    action: 'DNS verification',
    detail: 'Verified MX, SPF, DKIM and DMARC records',
  },
  {
    id: 'au-2',
    time: atTime(60),
    actor: 'admin',
    action: 'Mailbox created',
    detail: 'ops@harbor.co (Ops Team)',
  },
  {
    id: 'au-3',
    time: atTime(20),
    actor: 'admin',
    action: 'Forwarder added',
    detail: 'alex@harbor.co → alexmorgan@example.com',
  },
]

export const dnsRecords: { id: keyof DnsStatus; name: string; value: string }[] = [
  { id: 'mx', name: 'MX', value: 'harbor.co. 300 IN MX 10 mail.harbor.co.' },
  { id: 'spf', name: 'SPF', value: 'harbor.co. TXT "v=spf1 include:harbor.co ~all"' },
  {
    id: 'dkim',
    name: 'DKIM',
    value:
      'harbor-mail._domainkey.harbor.co. TXT "v=DKIM1; k=rsa; p=MIGfMA0GCSqGSIb3DQEBAQUAA4GNADCBiQKBgQC…"',
  },
  {
    id: 'dmarc',
    name: 'DMARC',
    value: '_dmarc.harbor.co. TXT "v=DMARC1; p=quarantine; rua=mailto:dmarc@harbor.co"',
  },
]

const read = <T>(key: string, seed: T): T => {
  try {
    const raw = localStorage.getItem(key)
    if (!raw) return seed
    const parsed = JSON.parse(raw) as T
    return parsed ?? seed
  } catch {
    return seed
  }
}

const readArray = <T>(key: string, seed: T[]): T[] => {
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

const clone = <T>(value: T): T =>
  Array.isArray(value) ? (value.map((item) => ({ ...item })) as T) : { ...value }

const roleOf = (mailbox: MailboxAccount): Role => {
  if (mailbox.email.toLowerCase() === 'alex@harbor.co') return 'owner'
  return mailbox.role === 'admin' ? 'admin' : 'member'
}

export const adminApi = {
  listMailboxes(): MailboxAccount[] {
    return readArray<MailboxAccount>(mailboxesKey, seedMailboxes).map((mailbox) => ({
      ...mailbox,
      role: roleOf(mailbox),
      quotaGB: Number.isFinite(mailbox.quotaGB) && mailbox.quotaGB > 0 ? mailbox.quotaGB : 10,
    }))
  },
  saveMailboxes(next: MailboxAccount[]) {
    write(mailboxesKey, next)
  },
  addMailbox(input: { email: string; displayName?: string; quotaGB?: number }) {
    const current = this.listMailboxes()
    const raw = input.email.trim().toLowerCase()
    const email = raw.includes('@') ? raw : `${raw}@harbor.co`
    if (
      !/^[a-z0-9.+-]+@[a-z0-9.-]+\.[a-z]{2,}$/.test(email) ||
      current.some((mailbox) => mailbox.email.toLowerCase() === email)
    )
      return current
    const next = [
      ...current,
      {
        id: `mb-${Date.now()}`,
        email,
        displayName: input.displayName?.trim() || email.split('@')[0],
        status: 'active' as const,
        role: 'member' as const,
        storageUsedGB: 0,
        quotaGB: input.quotaGB ?? 10,
      },
    ]
    this.saveMailboxes(next)
    this.logAudit('Mailbox created', `${email} (${next[next.length - 1].displayName})`)
    return next
  },
  removeMailbox(id: string) {
    const current = this.listMailboxes()
    const target = current.find((mailbox) => mailbox.id === id)
    if (!target || target.email.startsWith('alex@')) return current
    const next = current.filter((mailbox) => mailbox.id !== id)
    this.saveMailboxes(next)
    this.logAudit('Mailbox removed', target.email)
    return next
  },

  setMailboxStatus(id: string, status: MailboxStatus) {
    const current = this.listMailboxes()
    const target = current.find((mailbox) => mailbox.id === id)
    const next = current.map((mailbox) => (mailbox.id === id ? { ...mailbox, status } : mailbox))
    this.saveMailboxes(next)
    if (target)
      this.logAudit(`${status.charAt(0).toUpperCase() + status.slice(1)} mailbox`, target.email)
    return next
  },
  setMailboxQuota(id: string, quotaGB: number) {
    const current = this.listMailboxes()
    const target = current.find((mailbox) => mailbox.id === id)
    const clamped = Math.max(1, Math.min(100, Math.round(quotaGB)))
    const next = current.map((mailbox) =>
      mailbox.id === id ? { ...mailbox, quotaGB: clamped } : mailbox,
    )
    this.saveMailboxes(next)
    if (target && clamped !== target.quotaGB)
      this.logAudit('Quota changed', `${target.email} → ${clamped} GB`)
    return next
  },
  setRole(id: string, role: Role) {
    const current = this.listMailboxes()
    const target = current.find((mailbox) => mailbox.id === id)
    if (!target || target.role === role) return current
    const isPrimary = target.email.toLowerCase() === 'alex@harbor.co'
    if (isPrimary && role !== 'owner') return current
    if (!isPrimary && role === 'owner') return current
    const next = current.map((mailbox) => (mailbox.id === id ? { ...mailbox, role } : mailbox))
    this.saveMailboxes(next)
    this.logAudit('Role changed', `${target.email} → ${role}`)
    return next
  },
  resetPassword(id: string) {
    const current = this.listMailboxes()
    const target = current.find((mailbox) => mailbox.id === id)
    if (!target) return ''
    const temp = `hap-${Math.random().toString(36).slice(2, 6)}${Math.random().toString(36).slice(2, 6)}`
    const passwords = read<Record<string, string>>(passwordsKey, {})
    write(passwordsKey, { ...passwords, [target.email]: temp })
    this.logAudit('Password reset', target.email)
    return temp
  },
  tempPassword(email: string): string {
    return read<Record<string, string>>(passwordsKey, {})[email] ?? ''
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
    if (current.some((alias) => alias.address.toLowerCase() === address)) return current
    const next = [...current, { id: `al-${Date.now()}`, address, forwardTo }]
    this.saveAliases(next)
    this.logAudit('Alias created', `${address} → ${forwardTo}`)
    return next
  },
  removeAlias(id: string) {
    const current = this.listAliases()
    const target = current.find((alias) => alias.id === id)
    const next = current.filter((alias) => alias.id !== id)
    this.saveAliases(next)
    if (target) this.logAudit('Alias removed', target.address)
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
    if (
      !trimmed.includes('@') ||
      current.some((forwarder) => forwarder.from === from && forwarder.to === trimmed)
    )
      return current
    const next = [...current, { id: `fw-${Date.now()}`, from, to: trimmed, enabled: true }]
    this.saveForwarders(next)
    this.logAudit('Forwarder added', `${from} → ${trimmed}`)
    return next
  },
  toggleForwarder(id: string) {
    const current = this.listForwarders()
    const target = current.find((forwarder) => forwarder.id === id)
    const next = current.map((forwarder) =>
      forwarder.id === id ? { ...forwarder, enabled: !forwarder.enabled } : forwarder,
    )
    this.saveForwarders(next)
    if (target)
      this.logAudit(target.enabled ? 'Forwarder paused' : 'Forwarder enabled', target.from)
    return next
  },
  removeForwarder(id: string) {
    const current = this.listForwarders()
    const target = current.find((forwarder) => forwarder.id === id)
    const next = current.filter((forwarder) => forwarder.id !== id)
    this.saveForwarders(next)
    if (target) this.logAudit('Forwarder removed', target.from)
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
    this.logAudit('DNS verification', 'Verified MX, SPF, DKIM and DMARC records')
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

  getSecurity(): SecuritySettings {
    return { ...seedSecurity, ...read<SecuritySettings>(securityKey, seedSecurity) }
  },
  saveSecurity(next: SecuritySettings) {
    write(securityKey, next)
  },
  getSso(): SsoSettings {
    return { ...seedSso, ...read<SsoSettings>(ssoKey, seedSso) }
  },
  saveSso(next: SsoSettings) {
    write(ssoKey, next)
  },
  addBlockedSender(email: string) {
    const current = this.getSecurity()
    const trimmed = email.trim().toLowerCase()
    if (!trimmed.includes('@') || current.blockedSenders.includes(trimmed)) return current
    const next = { ...current, blockedSenders: [...current.blockedSenders, trimmed] }
    this.saveSecurity(next)
    this.logAudit('Blocked sender added', trimmed)
    return next
  },
  removeBlockedSender(email: string) {
    const current = this.getSecurity()
    const next = {
      ...current,
      blockedSenders: current.blockedSenders.filter((sender) => sender !== email),
    }
    this.saveSecurity(next)
    this.logAudit('Blocked sender removed', email)
    return next
  },

  listQuarantine(): QuarantinedMail[] {
    return readArray<QuarantinedMail>(quarantineKey, seedQuarantine)
  },
  saveQuarantine(next: QuarantinedMail[]) {
    write(quarantineKey, next)
  },
  async releaseQuarantine(id: string) {
    const current = this.listQuarantine()
    const target = current.find((message) => message.id === id)
    if (target) {
      const existing = await mailboxApi.listFor(primaryAccountId)
      const mail: Mail = {
        id: `mb-q-${Date.now()}`,
        initials: target.from.split('@')[0].slice(0, 2).toUpperCase(),
        sender: target.from.split('@')[0],
        email: target.from,
        subject: target.subject,
        preview: `${target.subject} — released from quarantine (${target.reason}).`,
        time: 'Just now',
        label: 'Inbox',
        color: '#5b6470',
        unread: true,
        accountId: primaryAccountId,
      }
      await mailboxApi.replaceFor(primaryAccountId, [mail, ...existing])
      this.logAudit('Quarantine released', `${target.subject} (${target.from})`)
    }
    const next = current.filter((message) => message.id !== id)
    this.saveQuarantine(next)
    return next
  },
  deleteQuarantine(id: string) {
    const current = this.listQuarantine()
    const target = current.find((message) => message.id === id)
    const next = current.filter((message) => message.id !== id)
    this.saveQuarantine(next)
    if (target) this.logAudit('Quarantine deleted', `${target.subject} (${target.from})`)
    return next
  },

  listAudit(): AuditEntry[] {
    return readArray<AuditEntry>(auditKey, seedAudit)
  },
  logAudit(action: string, detail: string) {
    const next = [
      { id: `au-${Date.now()}`, time: new Date().toISOString(), actor: 'admin', action, detail },
      ...this.listAudit(),
    ].slice(0, 50)
    write(auditKey, next)
    return next
  },
}
