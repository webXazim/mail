import { apiFetch } from '../lib/api'
import { mailboxApi } from './mailbox'
import { primaryAccountId } from './accounts'
import type { Mail } from '../types'

export type MailboxStatus = 'active' | 'quarantine' | 'disabled'

export type Role = 'owner' | 'admin' | 'member' | 'billing'

export type UserBusinessMembership = {
  organizationId: string
  organizationName: string
  role: 'owner' | 'admin' | 'member' | 'billing'
  membershipStatus: 'active' | 'invited' | 'suspended'
  planCode?: string | null
  planName?: string | null
  subscriptionStatus?: string | null
  assignedAt?: string | null
  currentPeriodStart?: string | null
  currentPeriodEnd?: string | null
  assignmentSource?: string | null
}

export type MailboxAccount = {
  id: string
  email: string
  displayName: string
  status: MailboxStatus
  role: Role
  storageUsedGB: number
  quotaGB: number
  plan?: string
  quotaSource?: 'plan' | 'override'
  planQuotaGB?: number
  providerQuotaGB?: number | null
  quotaInSync?: boolean
  platformRole?: 'user' | 'platform_support' | 'platform_admin'
  emailVerified?: boolean
  createdAt?: string
  lastActiveAt?: string | null
  businessCount?: number
  organizationId?: string | null
  organizationName?: string | null
  organizationRole?: 'owner' | 'admin' | 'member' | 'billing' | null
  subscriptionPlanCode?: string | null
  subscriptionPlanName?: string | null
  subscriptionStatus?: string | null
  subscriptionAssignedAt?: string | null
  subscriptionPeriodEnd?: string | null
  subscriptionAssignmentSource?: string | null
  businessMemberships?: UserBusinessMembership[]
}

export type Alias = {
  id: string
  address: string
  forwardTo: string
  destinationType?: 'mailbox' | 'external'
  enabled?: boolean
  syncStatus?: string
  syncError?: string
}

export type Forwarder = { id: string; from: string; to: string; enabled: boolean; verified?: boolean; keepCopy?: boolean }

export type DnsStatus = { mx: boolean; spf: boolean; dkim: boolean; dmarc: boolean }

export type DomainSettings = { domain: string; catchAllEnabled: boolean; catchAll: string; dnsZoneFile?: string; dnsManagement?: string; providerAvailable?: boolean }

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
  eventHash?: string
}

const mailboxesKey = 'cs-mail:admin:mailboxes'
const aliasesKey = 'cs-mail:admin:aliases'
const forwardersKey = 'cs-mail:admin:forwarders'
const dnsKey = 'cs-mail:admin:dns'
const domainKey = 'cs-mail:admin:domain'
const securityKey = 'cs-mail:admin:security'
const quarantineKey = 'cs-mail:admin:quarantine'
const auditKey = 'cs-mail:admin:audit'
const passwordsKey = 'cs-mail:admin:passwords'
const ssoKey = 'cs-mail:admin:sso'

export const seedMailboxes: MailboxAccount[] = [
  {
    id: 'mb-alex',
    email: 'alex@crescentsphere.com',
    displayName: 'Alex Morgan',
    status: 'active',
    role: 'owner',
    storageUsedGB: 0.4,
    quotaGB: 10,
  },
  {
    id: 'mb-nora',
    email: 'nora@crescentsphere.com',
    displayName: 'Nora Saleh',
    status: 'active',
    role: 'member',
    storageUsedGB: 0.2,
    quotaGB: 10,
  },
  {
    id: 'mb-jonas',
    email: 'jonas@crescentsphere.com',
    displayName: 'Jonas Meier',
    status: 'active',
    role: 'member',
    storageUsedGB: 0.1,
    quotaGB: 10,
  },
  {
    id: 'mb-dev',
    email: 'dev@crescentsphere.com',
    displayName: 'Dev Team',
    status: 'active',
    role: 'member',
    storageUsedGB: 0.1,
    quotaGB: 5,
  },
]

export const seedAliases: Alias[] = [
  { id: 'al-support', address: 'support@crescentsphere.com', forwardTo: 'alex@crescentsphere.com' },
  { id: 'al-sales', address: 'sales@crescentsphere.com', forwardTo: 'nora@crescentsphere.com' },
  { id: 'al-hello', address: 'hello@crescentsphere.com', forwardTo: 'alex@crescentsphere.com' },
]

export const seedForwarders: Forwarder[] = [
  { id: 'fw-alex', from: 'alex@crescentsphere.com', to: 'alexmorgan@example.com', enabled: true },
]

export const seedDomain: DomainSettings = {
  domain: 'crescentsphere.com',
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
  entityId: 'https://crescentsphere.com/saml2',
  enforce: false,
}

const atTime = (minutesAgo: number): string =>
  new Date(Date.now() - minutesAgo * 60_000).toISOString()

export const seedQuarantine: QuarantinedMail[] = [
  {
    id: 'q-1',
    from: 'prize@win-now.example',
    to: 'alex@crescentsphere.com',
    subject: 'You have won a prize!',
    date: atTime(35),
    reason: 'Blocked by global blocklist',
    sizeKB: 4,
  },
  {
    id: 'q-2',
    from: 'billing@ghost-invoice.example',
    to: 'nora@crescentsphere.com',
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
    detail: 'ops@crescentsphere.com (Ops Team)',
  },
  {
    id: 'au-3',
    time: atTime(20),
    actor: 'admin',
    action: 'Forwarder added',
    detail: 'alex@crescentsphere.com → alexmorgan@example.com',
  },
]

export const dnsRecords: { id: keyof DnsStatus; name: string; value: string }[] = [
  { id: 'mx', name: 'MX', value: 'crescentsphere.com. 300 IN MX 10 smtp.crescentsphere.com.' },
  { id: 'spf', name: 'SPF', value: 'crescentsphere.com. TXT "v=spf1 include:crescentsphere.com ~all"' },
  {
    id: 'dkim',
    name: 'DKIM',
    value:
      'cs-mail._domainkey.crescentsphere.com. TXT "v=DKIM1; k=rsa; p=MIGfMA0GCSqGSIb3DQEBAQUAA4GNADCBiQKBgQC…"',
  },
  {
    id: 'dmarc',
    name: 'DMARC',
    value: '_dmarc.crescentsphere.com. TXT "v=DMARC1; p=quarantine; rua=mailto:dmarc@crescentsphere.com"',
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
  if (mailbox.email.toLowerCase() === 'alex@crescentsphere.com') return 'owner'
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
    const email = raw.includes('@') ? raw : `${raw}@crescentsphere.com`
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
    const isPrimary = target.email.toLowerCase() === 'alex@crescentsphere.com'
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

const GB = 1024 * 1024 * 1024

type BackendUser = {
  id: string
  email: string
  display_name: string
  role: 'admin' | 'member' | 'billing'
  status: 'active' | 'suspended'
  plan: string
  quota_bytes: number
  quota_override_bytes?: number | null
  quota_source?: 'plan' | 'override'
  plan_mailbox_bytes?: number
  provider_quota_bytes?: number | null
  quota_in_sync?: boolean
  mail_account_id: string | null
  onboarded: boolean
  created_at: string
  storage_used_bytes: number
  storage_pct: number
  platform_role: 'user' | 'platform_support' | 'platform_admin'
  email_verified: boolean
  email_verified_at?: string | null
  last_active_at?: string | null
  business_count?: number
  primary_organization_id?: string | null
  primary_organization_name?: string | null
  primary_organization_role?: 'owner' | 'admin' | 'member' | 'billing' | null
  subscription_plan_code?: string | null
  subscription_plan_name?: string | null
  subscription_status?: string | null
  subscription_assigned_at?: string | null
  subscription_period_end?: string | null
  subscription_assignment_source?: string | null
  business_memberships?: Array<{
    organization_id: string
    organization_name: string
    role: 'owner' | 'admin' | 'member' | 'billing'
    membership_status: 'active' | 'invited' | 'suspended'
    plan_code?: string | null
    plan_name?: string | null
    subscription_status?: string | null
    assigned_at?: string | null
    current_period_start?: string | null
    current_period_end?: string | null
    assignment_source?: string | null
  }>
}

type BackendAlias = {
  id: string
  address: string
  domain: string
  source: string
  forwardTo: string
  destinationType?: 'mailbox' | 'external'
  enabled?: boolean
  syncStatus?: string
  syncError?: string
}

type BackendAudit = {
  id: string
  time: string
  actor: string
  action: string
  detail: unknown
  eventHash?: string
}

export type RemoteUsersPage = {
  users: MailboxAccount[]
  total: number
  limit: number
  offset: number
}

export type RemoteOverview = {
  adminEmail: string
  domain: string
  userCount: number
  adminCount: number
  suspendedUserCount: number
  unverifiedUserCount: number
  auditEventCount: number
  businessCount: number
  activeSubscriptionCount: number
  pastDueSubscriptionCount: number
  suspendedSubscriptionCount: number
  expiring30DaysCount: number
  paymentReviewCount: number
  providerHealthy: boolean
}

export type AdminSecurityPolicy = SecuritySettings & {
  providerAvailable?: boolean
  supported?: {
    spamThreshold: boolean
    retention: boolean
    blockedSenders: boolean
    requireTls: boolean
    scanAttachments: boolean
    dmarcPolicy: boolean
    sso: boolean
  }
}

export type AdminQueueMessage = {
  id: string
  createdAt?: string
  nextRetry?: string | null
  returnPath?: string
  recipients?: Record<string, unknown>
  size?: number
  priority?: number
}

export type AdminDiagnostics = {
  database: boolean
  mailProvider: boolean
  queueTotal: number
  automationErrors: number
  addressSyncErrors: number
  provisioning: { status: string; count: number }[]
}

export type LaunchCertification = {
  id: string
  release_label: string
  release_sha256: string
  environment: string
  status: 'running' | 'passed' | 'failed' | 'aborted'
  report_sha256: string
  report_path: string
  mandatory_passed: number
  mandatory_failed: number
  optional_skipped: number
  started_at: string
  completed_at?: string | null
  created_at: string
}

const remoteRole = (role: BackendUser['role']): Role =>
  role === 'admin' ? 'admin' : role === 'billing' ? 'billing' : 'member'

const mapUser = (user: BackendUser): MailboxAccount => ({
  id: user.id,
  email: user.email,
  displayName: user.display_name,
  status: user.status === 'suspended' ? 'disabled' : 'active',
  role: remoteRole(user.role),
  storageUsedGB: (user.storage_used_bytes ?? 0) / GB,
  quotaGB: Math.max(1, Math.round((user.quota_bytes ?? 0) / GB)),
  plan: user.plan,
  quotaSource: user.quota_source ?? (user.quota_override_bytes ? 'override' : 'plan'),
  planQuotaGB: Math.max(1, Math.round((user.plan_mailbox_bytes ?? user.quota_bytes ?? 0) / GB)),
  providerQuotaGB: user.provider_quota_bytes ? user.provider_quota_bytes / GB : null,
  quotaInSync: user.quota_in_sync,
  platformRole: user.platform_role,
  emailVerified: Boolean(user.email_verified),
  createdAt: user.created_at,
  lastActiveAt: user.last_active_at ?? null,
  businessCount: user.business_count ?? 0,
  organizationId: user.primary_organization_id ?? null,
  organizationName: user.primary_organization_name ?? null,
  organizationRole: user.primary_organization_role ?? null,
  subscriptionPlanCode: user.subscription_plan_code ?? null,
  subscriptionPlanName: user.subscription_plan_name ?? null,
  subscriptionStatus: user.subscription_status ?? null,
  subscriptionAssignedAt: user.subscription_assigned_at ?? null,
  subscriptionPeriodEnd: user.subscription_period_end ?? null,
  subscriptionAssignmentSource: user.subscription_assignment_source ?? null,
  businessMemberships: (user.business_memberships ?? []).map((membership) => ({
    organizationId: membership.organization_id,
    organizationName: membership.organization_name,
    role: membership.role,
    membershipStatus: membership.membership_status,
    planCode: membership.plan_code ?? null,
    planName: membership.plan_name ?? null,
    subscriptionStatus: membership.subscription_status ?? null,
    assignedAt: membership.assigned_at ?? null,
    currentPeriodStart: membership.current_period_start ?? null,
    currentPeriodEnd: membership.current_period_end ?? null,
    assignmentSource: membership.assignment_source ?? null,
  })),
})

const detailText = (detail: unknown): string => {
  if (typeof detail === 'string') return detail
  if (detail === null || detail === undefined) return ''
  try {
    return JSON.stringify(detail)
  } catch {
    return String(detail)
  }
}

const mapAudit = (entry: BackendAudit): AuditEntry => ({
  id: entry.id,
  time: entry.time,
  actor: entry.actor,
  action: entry.action,
  detail: detailText(entry.detail),
  eventHash: entry.eventHash,
})

export type CreateUserInput = {
  email: string
  displayName: string
  password: string
  quotaGB?: number
}

/** Live server-authoritative Admin center. Demo mode keeps the localStorage
 * mock above, but authenticated remote mode never falls back to browser state. */
export const remoteAdminApi = {
  async overview(): Promise<RemoteOverview> {
    const data = await apiFetch<{
      admin: { id: string; email: string }
      domain: string
      user_count: number
      admin_count: number
      suspended_user_count: number
      unverified_user_count: number
      audit_events: number
      business_count: number
      active_subscription_count: number
      past_due_subscription_count: number
      suspended_subscription_count: number
      expiring_30_days_count: number
      payment_review_count: number
      provider_healthy: boolean
    }>('/api/admin/overview')
    return {
      adminEmail: data.admin.email,
      domain: data.domain,
      userCount: data.user_count ?? 0,
      adminCount: data.admin_count ?? 0,
      suspendedUserCount: data.suspended_user_count ?? 0,
      unverifiedUserCount: data.unverified_user_count ?? 0,
      auditEventCount: data.audit_events ?? 0,
      businessCount: data.business_count ?? 0,
      activeSubscriptionCount: data.active_subscription_count ?? 0,
      pastDueSubscriptionCount: data.past_due_subscription_count ?? 0,
      suspendedSubscriptionCount: data.suspended_subscription_count ?? 0,
      expiring30DaysCount: data.expiring_30_days_count ?? 0,
      paymentReviewCount: data.payment_review_count ?? 0,
      providerHealthy: Boolean(data.provider_healthy),
    }
  },

  async usersPage(options: { q?: string; status?: 'active' | 'suspended'; platformRole?: 'user' | 'platform_support' | 'platform_admin'; limit?: number; offset?: number } = {}): Promise<RemoteUsersPage> {
    const params = new URLSearchParams()
    if (options.q?.trim()) params.set('q', options.q.trim())
    if (options.status) params.set('status', options.status)
    if (options.platformRole) params.set('platform_role', options.platformRole)
    params.set('limit', String(options.limit ?? 100))
    params.set('offset', String(options.offset ?? 0))
    const data = await apiFetch<{ users: BackendUser[]; total?: number; limit?: number; offset?: number }>(`/api/admin/users?${params.toString()}`)
    return {
      users: (data.users ?? []).map(mapUser),
      total: data.total ?? data.users?.length ?? 0,
      limit: data.limit ?? options.limit ?? 100,
      offset: data.offset ?? options.offset ?? 0,
    }
  },

  async users(): Promise<MailboxAccount[]> {
    return (await this.usersPage()).users
  },

  async createUser(input: CreateUserInput): Promise<void> {
    await apiFetch('/api/admin/users', {
      method: 'POST',
      body: JSON.stringify({
        email: input.email,
        display_name: input.displayName,
        password: input.password,
        role: 'member',
        quota_bytes: input.quotaGB ? Math.round(input.quotaGB * GB) : undefined,
      }),
    })
  },

  async updateUser(
    id: string,
    patch: { display_name?: string; role?: string; platform_role?: 'user' | 'platform_support' | 'platform_admin'; status?: 'active' | 'suspended'; plan?: string; quota_bytes?: number; reset_quota_override?: boolean; password?: string },
  ): Promise<void> {
    await apiFetch(`/api/admin/users/${encodeURIComponent(id)}`, {
      method: 'PATCH',
      body: JSON.stringify(patch),
    })
  },

  async deleteUser(id: string): Promise<void> {
    await apiFetch(`/api/admin/users/${encodeURIComponent(id)}`, { method: 'DELETE' })
  },

  async aliases(): Promise<Alias[]> {
    const data = await apiFetch<{ aliases: BackendAlias[] }>('/api/admin/aliases')
    return (data.aliases ?? []).map((alias) => ({
      id: alias.id,
      address: alias.address,
      forwardTo: alias.forwardTo,
      destinationType: alias.destinationType,
      enabled: alias.enabled,
      syncStatus: alias.syncStatus,
      syncError: alias.syncError,
    }))
  },

  async createAlias(local: string, forwardTo: string, domain: string): Promise<void> {
    await apiFetch('/api/aliases', {
      method: 'POST',
      body: JSON.stringify({ domain, source: local, forwardTo }),
    })
  },

  async deleteAlias(id: string): Promise<void> {
    await apiFetch(`/api/aliases/${encodeURIComponent(id)}`, { method: 'DELETE' })
  },

  async forwarders(): Promise<Forwarder[]> {
    const data = await apiFetch<{ forwarders: Forwarder[] }>('/api/admin/forwarders')
    return data.forwarders ?? []
  },

  async createForwarder(from: string, to: string): Promise<{ verificationCode?: string | null; forwarder?: Forwarder }> {
    return await apiFetch<{ verificationCode?: string | null; forwarder?: Forwarder }>('/api/admin/forwarders', {
      method: 'POST',
      body: JSON.stringify({ from, to, keep_copy: true }),
    })
  },

  async verifyForwarder(id: string, code: string): Promise<void> {
    await apiFetch(`/api/admin/forwarders/${encodeURIComponent(id)}/verify`, {
      method: 'POST',
      body: JSON.stringify({ code }),
    })
  },

  async setForwarderEnabled(id: string, enabled: boolean): Promise<void> {
    await apiFetch(`/api/admin/forwarders/${encodeURIComponent(id)}`, {
      method: 'PATCH',
      body: JSON.stringify({ enabled }),
    })
  },

  async deleteForwarder(id: string): Promise<void> {
    await apiFetch(`/api/admin/forwarders/${encodeURIComponent(id)}`, { method: 'DELETE' })
  },

  async domain(): Promise<{ domain: DomainSettings; dns: DnsStatus }> {
    const data = await apiFetch<{
      domain: string
      catchAllEnabled: boolean
      catchAll: string
      dnsZoneFile?: string
      dnsManagement?: string
      providerAvailable?: boolean
      dns: DnsStatus
    }>('/api/admin/domain')
    return {
      domain: {
        domain: data.domain,
        catchAllEnabled: Boolean(data.catchAllEnabled),
        catchAll: data.catchAll ?? '',
        dnsZoneFile: data.dnsZoneFile ?? '',
        dnsManagement: data.dnsManagement,
        providerAvailable: data.providerAvailable,
      },
      dns: data.dns ?? { mx: false, spf: false, dkim: false, dmarc: false },
    }
  },

  async updateDomain(patch: { catch_all_enabled?: boolean; catch_all?: string }): Promise<{ domain: DomainSettings; dns: DnsStatus }> {
    const data = await apiFetch<{
      domain: string
      catchAllEnabled: boolean
      catchAll: string
      dnsZoneFile?: string
      dnsManagement?: string
      providerAvailable?: boolean
      dns: DnsStatus
    }>('/api/admin/domain', { method: 'PATCH', body: JSON.stringify(patch) })
    return {
      domain: {
        domain: data.domain,
        catchAllEnabled: Boolean(data.catchAllEnabled),
        catchAll: data.catchAll ?? '',
        dnsZoneFile: data.dnsZoneFile ?? '',
        dnsManagement: data.dnsManagement,
        providerAvailable: data.providerAvailable,
      },
      dns: data.dns ?? { mx: false, spf: false, dkim: false, dmarc: false },
    }
  },

  async securityPolicy(): Promise<AdminSecurityPolicy> {
    return await apiFetch<AdminSecurityPolicy>('/api/admin/security-policy')
  },

  async updateSecurityPolicy(patch: Partial<Pick<SecuritySettings, 'spamThreshold' | 'retentionDays' | 'trashAutoPurge'>>): Promise<AdminSecurityPolicy> {
    return await apiFetch<AdminSecurityPolicy>('/api/admin/security-policy', {
      method: 'PATCH',
      body: JSON.stringify(patch),
    })
  },

  async quarantine(): Promise<QuarantinedMail[]> {
    const data = await apiFetch<{ messages: QuarantinedMail[] }>('/api/admin/quarantine?limit=100')
    return data.messages ?? []
  },

  async releaseQuarantine(id: string): Promise<void> {
    await apiFetch(`/api/admin/quarantine/${encodeURIComponent(id)}/release`, { method: 'POST' })
  },

  async deleteQuarantine(id: string): Promise<void> {
    await apiFetch(`/api/admin/quarantine/${encodeURIComponent(id)}`, { method: 'DELETE' })
  },

  async queue(): Promise<{ messages: AdminQueueMessage[]; total: number }> {
    return await apiFetch<{ messages: AdminQueueMessage[]; total: number }>('/api/admin/queue?limit=100')
  },

  async retryQueuedMessage(id: string): Promise<void> {
    await apiFetch(`/api/admin/queue/${encodeURIComponent(id)}/retry`, { method: 'POST' })
  },

  async cancelQueuedMessage(id: string): Promise<void> {
    await apiFetch(`/api/admin/queue/${encodeURIComponent(id)}`, { method: 'DELETE' })
  },

  async diagnostics(): Promise<AdminDiagnostics> {
    return await apiFetch<AdminDiagnostics>('/api/admin/diagnostics')
  },

  async launchCertifications(): Promise<LaunchCertification[]> {
    const data = await apiFetch<{ runs: LaunchCertification[] }>('/api/admin/launch-certifications')
    return data.runs ?? []
  },

  async blockedSenders(): Promise<string[]> {
    const data = await apiFetch<{ suppressions: { email: string }[] }>('/api/admin/suppressions')
    return (data.suppressions ?? []).map((entry) => entry.email)
  },

  async blockSender(email: string): Promise<void> {
    await apiFetch('/api/admin/suppressions', {
      method: 'POST',
      body: JSON.stringify({ email, reason: 'Blocked by admin' }),
    })
  },

  async unblockSender(email: string): Promise<void> {
    await apiFetch(`/api/admin/suppressions/${encodeURIComponent(email)}`, { method: 'DELETE' })
  },

  async audit(): Promise<AuditEntry[]> {
    const data = await apiFetch<{ entries: BackendAudit[] }>('/api/admin/audit')
    return (data.entries ?? []).map(mapAudit)
  },

  /** Download the full admin audit trail as an RFC 4180 CSV document. */
  async auditExport(): Promise<string> {
    return await apiFetch<string>('/api/admin/audit/export')
  },
}

export type PlatformControls = {
  public_signup_enabled: boolean
  business_creation_enabled: boolean
  plan_ordering_enabled: boolean
  domain_onboarding_enabled: boolean
  mailbox_provisioning_enabled: boolean
  outbound_sending_enabled: boolean
  maintenance_message: string
  updated_at?: string
  updated_by?: string | null
  updated_by_email?: string | null
}

export type AdminBusiness = {
  id: string
  name: string
  slug: string
  status: 'active' | 'suspended' | 'closed'
  status_reason: string
  status_changed_at?: string | null
  owner_email?: string | null
  member_count: number
  domain_count: number
  mailbox_count: number
  active_mailbox_count: number
  plan_code?: string | null
  plan_name?: string | null
  subscription_status?: string | null
  purchased_mailbox_count?: number | null
  current_period_end?: string | null
  assignment_source?: string | null
  created_at: string
}

export type AdminBusinessMember = {
  user_id: string
  email: string
  display_name: string
  role: 'owner' | 'admin' | 'billing' | 'member'
  status: 'active' | 'suspended'
  platform_role: 'user' | 'platform_support' | 'platform_admin'
  joined_at: string
}

export type AdminHostedDomain = {
  id: string
  organization_id: string
  organization_name: string
  domain: string
  status: string
  is_primary: boolean
  verified_at?: string | null
  provider_domain_id?: string | null
  dns_ready: boolean
  dns: { mx: boolean; spf: boolean; dkim: boolean; dmarc: boolean }
  last_error: string
  last_dns_readiness_check?: string | null
  mailbox_count: number
  created_at: string
}

export type AdminHostedMailbox = {
  id: string
  organization_id: string
  organization_name: string
  domain: string
  address: string
  display_name: string
  status: string
  sync_status: string
  sync_error: string
  quota_bytes: number
  quota_used?: number | null
  provider_account_id?: string | null
  user_id?: string | null
  user_email?: string | null
  created_at: string
}

export type AdminRecoveryItem = {
  id: string
  kind: 'provisioning' | 'import' | 'scheduled' | 'billing_email'
  operation?: string
  target: string
  status: string
  attempts: number
  max_attempts?: number
  last_error: string
  updated_at: string
  mailbox_id?: string | null
}

export type AdminRecovery = {
  provisioning: AdminRecoveryItem[]
  imports: AdminRecoveryItem[]
  scheduled: AdminRecoveryItem[]
  billing_email: AdminRecoveryItem[]
}

const listParams = (options: { q?: string; status?: string; limit?: number; offset?: number } = {}) => {
  const params = new URLSearchParams()
  if (options.q?.trim()) params.set('q', options.q.trim())
  if (options.status?.trim()) params.set('status', options.status.trim())
  params.set('limit', String(options.limit ?? 50))
  params.set('offset', String(options.offset ?? 0))
  return params.toString()
}

export const platformAdminApi = {
  controls(): Promise<PlatformControls> {
    return apiFetch<PlatformControls>('/api/admin/platform-controls')
  },
  updateControls(value: PlatformControls): Promise<PlatformControls> {
    return apiFetch<PlatformControls>('/api/admin/platform-controls', {
      method: 'PUT',
      body: JSON.stringify(value),
    })
  },
  businesses(options: { q?: string; status?: string; limit?: number; offset?: number } = {}) {
    return apiFetch<{ businesses: AdminBusiness[]; total: number; limit: number; offset: number }>(`/api/admin/businesses?${listParams(options)}`)
  },
  updateBusinessStatus(id: string, input: { status: 'active' | 'suspended' | 'closed'; reason?: string; confirm_name?: string }) {
    return apiFetch(`/api/admin/businesses/${encodeURIComponent(id)}/status`, { method: 'PATCH', body: JSON.stringify(input) })
  },
  businessMembers(id: string) {
    return apiFetch<{ members: AdminBusinessMember[] }>(`/api/admin/businesses/${encodeURIComponent(id)}/members`)
  },
  updateBusinessMember(organizationId: string, userId: string, input: { role: AdminBusinessMember['role']; status: AdminBusinessMember['status'] }) {
    return apiFetch(`/api/admin/businesses/${encodeURIComponent(organizationId)}/members/${encodeURIComponent(userId)}`, { method: 'PATCH', body: JSON.stringify(input) })
  },
  removeBusinessMember(organizationId: string, userId: string) {
    return apiFetch(`/api/admin/businesses/${encodeURIComponent(organizationId)}/members/${encodeURIComponent(userId)}`, { method: 'DELETE' })
  },
  domains(options: { q?: string; status?: string; limit?: number; offset?: number } = {}) {
    return apiFetch<{ domains: AdminHostedDomain[]; total: number; limit: number; offset: number }>(`/api/admin/hosted-domains?${listParams(options)}`)
  },
  domainAction(id: string, action: 'suspend' | 'resume' | 'check_dns' | 'provision' | 'delete', confirmDomain = '') {
    return apiFetch(`/api/admin/hosted-domains/${encodeURIComponent(id)}/action`, { method: 'POST', body: JSON.stringify({ action, confirm_domain: confirmDomain }) })
  },
  mailboxes(options: { q?: string; status?: string; limit?: number; offset?: number } = {}) {
    return apiFetch<{ mailboxes: AdminHostedMailbox[]; total: number; limit: number; offset: number }>(`/api/admin/hosted-mailboxes?${listParams(options)}`)
  },
  mailboxAction(
    id: string,
    action: 'suspend' | 'activate' | 'delete' | 'set_quota',
    options: { confirmAddress?: string; quotaBytes?: number; resetToDefault?: boolean } = {},
  ) {
    return apiFetch(`/api/admin/hosted-mailboxes/${encodeURIComponent(id)}/action`, {
      method: 'POST',
      body: JSON.stringify({
        action,
        confirm_address: options.confirmAddress ?? '',
        quota_bytes: options.quotaBytes,
        reset_to_default: Boolean(options.resetToDefault),
      }),
    })
  },
  recovery(): Promise<AdminRecovery> {
    return apiFetch<AdminRecovery>('/api/admin/recovery')
  },
  retryRecovery(id: string, kind: AdminRecoveryItem['kind']) {
    return apiFetch(`/api/admin/recovery/${encodeURIComponent(id)}/retry`, { method: 'POST', body: JSON.stringify({ kind }) })
  },
}
