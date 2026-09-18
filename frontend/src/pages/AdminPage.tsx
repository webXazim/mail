import { useEffect, useState, type FormEvent } from 'react'
import { useNavigate } from 'react-router-dom'
import {
  AtSign,
  Check,
  Copy,
  CreditCard,
  FileWarning,
  Globe,
  History as HistoryIcon,
  KeyRound,
  LayoutDashboard,
  Mailbox,
  RefreshCw,
  Search,
  ShieldAlert,
  ShieldCheck,
  Trash2,
  UserCog,
} from 'lucide-react'
import {
  adminApi,
  dnsRecords,
  seedSso,
  ssoProviders,
  type Alias,
  type AuditEntry,
  type MailboxAccount,
  type QuarantinedMail,
  type SecuritySettings,
  type SsoSettings,
} from '../services/admin'
import { identitiesApi } from '../services/identities'
import { useMail } from '../state/mail/MailContext'

type AdminTab =
  | 'overview'
  | 'mailboxes'
  | 'roles'
  | 'aliases'
  | 'forwarders'
  | 'domain'
  | 'security'
  | 'quarantine'
  | 'audit'

const tabs: { id: AdminTab; label: string; icon: typeof LayoutDashboard }[] = [
  { id: 'overview', label: 'Overview', icon: LayoutDashboard },
  { id: 'mailboxes', label: 'Mailboxes', icon: Mailbox },
  { id: 'roles', label: 'Roles', icon: UserCog },
  { id: 'aliases', label: 'Aliases', icon: AtSign },
  { id: 'forwarders', label: 'Forwarders', icon: RefreshCw },
  { id: 'domain', label: 'Domain', icon: Globe },
  { id: 'security', label: 'Security', icon: ShieldCheck },
  { id: 'quarantine', label: 'Quarantine', icon: FileWarning },
  { id: 'audit', label: 'Audit log', icon: HistoryIcon },
]

const timeFmt = (iso: string) =>
  new Date(iso).toLocaleString([], {
    month: 'short',
    day: 'numeric',
    hour: 'numeric',
    minute: '2-digit',
  })

const retentionLabel = (days: number) => (days === 0 ? 'Forever' : `${days} days`)

const roleLabel: Record<MailboxAccount['role'], string> = {
  owner: 'Owner',
  admin: 'Admin',
  member: 'Member',
}

export function AdminPage() {
  const navigate = useNavigate()
  const { reload } = useMail()
  const [tab, setTab] = useState<AdminTab>('overview')
  const [mailboxes, setMailboxes] = useState<MailboxAccount[]>(() => adminApi.listMailboxes())
  const [aliases, setAliases] = useState<Alias[]>(() => adminApi.listAliases())
  const [forwarders, setForwarders] = useState(() => adminApi.listForwarders())
  const [dns, setDns] = useState(() => adminApi.getDns())
  const [domain, setDomain] = useState(() => adminApi.getDomainSettings())
  const [security, setSecurity] = useState<SecuritySettings>(() => adminApi.getSecurity())
  const [sso, setSso] = useState<SsoSettings>(() => adminApi.getSso())
  const [quarantine, setQuarantine] = useState<QuarantinedMail[]>(() => adminApi.listQuarantine())
  const [audit, setAudit] = useState<AuditEntry[]>(() => adminApi.listAudit())

  const [mailboxForm, setMailboxForm] = useState({ email: '', displayName: '', quotaGB: 10 })
  const [aliasForm, setAliasForm] = useState({ local: '', forwardTo: '' })
  const [forwarderForm, setForwarderForm] = useState({ from: mailboxes[0]?.email ?? '', to: '' })
  const [blockedForm, setBlockedForm] = useState('')
  const [mailboxQuery, setMailboxQuery] = useState('')
  const [mailboxStatusFilter, setMailboxStatusFilter] = useState<'all' | MailboxAccount['status']>(
    'all',
  )
  const [notice, setNotice] = useState('')
  const [copied, setCopied] = useState<string>('')

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') navigate('/mail/inbox')
    }
    window.addEventListener('keydown', handleKeyDown)
    return () => window.removeEventListener('keydown', handleKeyDown)
  }, [navigate])

  const showNotice = (message: string) => {
    setNotice(message)
    window.setTimeout(() => setNotice(''), 4000)
  }

  const dnsVerified = dns.mx && dns.spf && dns.dkim && dns.dmarc
  const activeCount = mailboxes.filter((mailbox) => mailbox.status === 'active').length
  const totalUsed = mailboxes.reduce((sum, mailbox) => sum + mailbox.storageUsedGB, 0)
  const totalQuota = mailboxes.reduce((sum, mailbox) => sum + mailbox.quotaGB, 0)
  const ownersCount = mailboxes.filter((mailbox) => mailbox.role === 'owner').length
  const adminsCount = mailboxes.filter((mailbox) => mailbox.role === 'admin').length
  const membersCount = mailboxes.filter((mailbox) => mailbox.role === 'member').length
  const visibleMailboxes = mailboxes.filter((mailbox) => {
    const query = mailboxQuery.trim().toLowerCase()
    const matchesQuery =
      !query || `${mailbox.email} ${mailbox.displayName}`.toLowerCase().includes(query)
    return matchesQuery && (mailboxStatusFilter === 'all' || mailbox.status === mailboxStatusFilter)
  })

  const addMailbox = (event: FormEvent) => {
    event.preventDefault()
    const raw = mailboxForm.email.trim().toLowerCase()
    const full = raw.includes('@') ? raw : `${raw}@harbor.co`
    const current = adminApi.addMailbox({
      email: raw,
      displayName: mailboxForm.displayName,
      quotaGB: mailboxForm.quotaGB,
    })
    if (current === mailboxes) {
      showNotice('That mailbox already exists')
      return
    }
    setMailboxes(current)
    identitiesApi.add({ email: raw, displayName: mailboxForm.displayName })
    setMailboxForm({ email: '', displayName: '', quotaGB: 10 })
    showNotice(`Mailbox ${full} created`)
  }

  const removeMailbox = (id: string) => {
    const target = mailboxes.find((mailbox) => mailbox.id === id)
    const next = adminApi.removeMailbox(id)
    if (next === mailboxes) {
      showNotice("The primary mailbox can't be removed")
      return
    }
    setMailboxes(next)
    setForwarderForm((current) => ({
      ...current,
      from: current.from === target?.email ? (next[0]?.email ?? '') : current.from,
    }))
    setAudit(adminApi.listAudit())
    showNotice('Mailbox removed')
  }

  const changeStatus = (id: string, status: string) => {
    const next = adminApi.setMailboxStatus(id, status as MailboxAccount['status'])
    setMailboxes(next)
    setAudit(adminApi.listAudit())
  }

  const changeQuota = (id: string, value: string) => {
    setMailboxes(adminApi.setMailboxQuota(id, Number(value) || 1))
  }

  const changeRole = (id: string, role: string) => {
    const next = adminApi.setRole(id, role as MailboxAccount['role'])
    if (next === mailboxes) {
      showNotice("That role can't be assigned here")
      return
    }
    setMailboxes(next)
    setAudit(adminApi.listAudit())
    showNotice('Role updated')
  }

  const resetPassword = (id: string) => {
    const target = mailboxes.find((mailbox) => mailbox.id === id)
    const temp = adminApi.resetPassword(id)
    setAudit(adminApi.listAudit())
    showNotice(`Temporary password for ${target?.email} is ${temp}`)
  }

  const addAlias = (event: FormEvent) => {
    event.preventDefault()
    if (!aliasForm.forwardTo) return
    const next = adminApi.addAlias(aliasForm.local, aliasForm.forwardTo, domain.domain)
    if (next === aliases) {
      showNotice('That alias already exists')
      return
    }
    setAliases(next)
    setAudit(adminApi.listAudit())
    setAliasForm({ local: '', forwardTo: '' })
    showNotice(`Alias ${aliasForm.local.trim().toLowerCase()}@${domain.domain} created`)
  }

  const removeAlias = (id: string) => {
    setAliases(adminApi.removeAlias(id))
    setAudit(adminApi.listAudit())
  }

  const addForwarder = (event: FormEvent) => {
    event.preventDefault()
    const next = adminApi.addForwarder(forwarderForm.from, forwarderForm.to)
    if (next === forwarders) {
      showNotice('That forwarder already exists or the address is invalid')
      return
    }
    setForwarders(next)
    setAudit(adminApi.listAudit())
    setForwarderForm((current) => ({ ...current, to: '' }))
    showNotice('Forwarder created')
  }

  const toggleForwarder = (id: string) => {
    setForwarders(adminApi.toggleForwarder(id))
    setAudit(adminApi.listAudit())
  }
  const removeForwarder = (id: string) => {
    setForwarders(adminApi.removeForwarder(id))
    setAudit(adminApi.listAudit())
  }

  const verifyRecords = () => {
    setDns(adminApi.verifyAll())
    setAudit(adminApi.listAudit())
    showNotice('All records verified')
  }

  const copyRecord = async (record: { id: string; value: string }) => {
    try {
      await navigator.clipboard.writeText(record.value)
      setCopied(record.id)
      window.setTimeout(() => setCopied(''), 1800)
    } catch {
      showNotice('Copy not available in this browser')
    }
  }

  const setCatchAllEnabled = (enabled: boolean) => {
    const next = {
      ...domain,
      catchAllEnabled: enabled,
      catchAll: enabled ? domain.catchAll || mailboxes[0]?.email || '' : domain.catchAll,
    }
    setDomain(next)
    adminApi.saveDomainSettings(next)
    adminApi.logAudit(enabled ? 'Catch-all enabled' : 'Catch-all disabled', domain.domain)
    setAudit(adminApi.listAudit())
  }

  const setCatchAllTarget = (target: string) => {
    const next = { ...domain, catchAll: target, catchAllEnabled: true }
    setDomain(next)
    adminApi.saveDomainSettings(next)
    adminApi.logAudit('Catch-all target changed', target)
    setAudit(adminApi.listAudit())
  }

  const updateSecurity = (patch: Partial<SecuritySettings>) => {
    const next = { ...security, ...patch }
    setSecurity(next)
    adminApi.saveSecurity(next)
    return next
  }

  const updateSecurityAndLog = (
    patch: Partial<SecuritySettings>,
    action: string,
    detail: string,
  ) => {
    updateSecurity(patch)
    adminApi.logAudit(action, detail)
    setAudit(adminApi.listAudit())
  }

  const updateSso = (patch: Partial<SsoSettings>) => {
    const next = { ...sso, ...patch }
    setSso(next)
    adminApi.saveSso(next)
    return next
  }

  const updateSsoAndLog = (patch: Partial<SsoSettings>, action: string, detail: string) => {
    updateSso(patch)
    adminApi.logAudit(action, detail)
    setAudit(adminApi.listAudit())
  }

  const addBlockedSender = (event: FormEvent) => {
    event.preventDefault()
    const next = adminApi.addBlockedSender(blockedForm)
    if (next === security) {
      showNotice('That sender is already blocked')
      return
    }
    setSecurity(next)
    setAudit(adminApi.listAudit())
    setBlockedForm('')
    showNotice('Sender blocked')
  }

  const removeBlockedSender = (email: string) => {
    setSecurity(adminApi.removeBlockedSender(email))
    setAudit(adminApi.listAudit())
  }

  const releaseQuarantine = async (id: string) => {
    const next = await adminApi.releaseQuarantine(id)
    setQuarantine(next)
    setAudit(adminApi.listAudit())
    void reload()
    showNotice('Message released — delivered to Inbox')
  }

  const removeQuarantine = (id: string) => {
    setQuarantine(adminApi.deleteQuarantine(id))
    setAudit(adminApi.listAudit())
    showNotice('Quarantined message deleted')
  }

  const recordValue = (record: { id: string; value: string }) =>
    record.id === 'dmarc'
      ? `_dmarc.${domain.domain}. TXT "v=DMARC1; p=${security.dmarcPolicy}; rua=mailto:dmarc@${domain.domain}"`
      : record.value
  const dmarcLabel = { none: 'No action', quarantine: 'Quarantine', reject: 'Reject' }[
    security.dmarcPolicy
  ]

  return (
    <div className="admin-page" role="region" aria-label="Admin center">
      <header className="calendar-head">
        <div>
          <p className="eyebrow">Workspace / Admin</p>
          <h1>Admin center</h1>
        </div>
        <div className="calendar-head__actions">
          <button
            type="button"
            className="secondary-button"
            onClick={() => navigate('/mail/admin/billing')}
          >
            <CreditCard size={14} />
            Payments &amp; plans
          </button>
        </div>
      </header>

      <nav className="admin-nav" aria-label="Admin sections">
        {tabs.map((tabItem) => {
          const Icon = tabItem.icon
          return (
            <button
              type="button"
              className={tab === tabItem.id ? 'admin-nav--active' : ''}
              aria-current={tab === tabItem.id ? 'page' : undefined}
              onClick={() => setTab(tabItem.id)}
              key={tabItem.id}
            >
              <Icon size={14} />
              {tabItem.label}
            </button>
          )
        })}
      </nav>

      {notice && <p className="settings-notice settings-notice--ok">{notice}</p>}

      {tab === 'overview' && (
        <>
          <div className="admin-stats">
            <div className="admin-stat">
              <strong>{activeCount}</strong>
              <small>Active mailboxes</small>
            </div>
            <div className="admin-stat">
              <strong>{adminsCount + ownersCount}</strong>
              <small>Owners &amp; admins</small>
            </div>
            <div className="admin-stat">
              <strong>{quarantine.length}</strong>
              <small>Quarantined messages</small>
            </div>
            <div className="admin-stat">
              <strong>{aliases.length}</strong>
              <small>Aliases</small>
            </div>
            <div className="admin-stat">
              <strong>{forwarders.length}</strong>
              <small>Forwarders</small>
            </div>
            <div className="admin-stat">
              <strong>
                {totalUsed.toFixed(1)} / {totalQuota} GB
              </strong>
              <small>Storage used</small>
            </div>
            <div className="admin-stat">
              <strong>{sso.enabled ? 'On' : 'Off'}</strong>
              <small>Single sign-on</small>
            </div>
          </div>

          <section className="settings-section">
            <h2>Domain health</h2>
            <div className="billing-plan">
              <div>
                <strong>{domain.domain}</strong>
                <small>
                  {dnsVerified
                    ? 'All records verified — mail is flowing.'
                    : `${Object.values(dns).filter(Boolean).length} of ${dnsRecords.length} records verified`}
                </small>
              </div>
              <div className="admin-actions">
                <button type="button" className="secondary-button" onClick={() => setTab('domain')}>
                  <Globe size={14} />
                  Open DNS records
                </button>
                <button
                  type="button"
                  className="secondary-button"
                  onClick={() => setTab('security')}
                >
                  <ShieldCheck size={14} />
                  Security policy
                </button>
              </div>
            </div>
          </section>

          <section className="settings-section">
            <h2>Recent activity</h2>
            {audit.slice(0, 5).map((entry) => (
              <div className="billing-row" key={entry.id}>
                <div>
                  <strong>{entry.action}</strong>
                  <small>{entry.detail}</small>
                </div>
                <span className="admin-audit-time">{timeFmt(entry.time)}</span>
              </div>
            ))}
            <button type="button" className="secondary-button" onClick={() => setTab('audit')}>
              <HistoryIcon size={14} />
              View full audit log
            </button>
          </section>
        </>
      )}

      {tab === 'mailboxes' && (
        <>
          <section className="settings-section">
            <div className="admin-section-head">
              <h2>Mailboxes</h2>
              <span className="admin-section-count">
                {visibleMailboxes.length} of {mailboxes.length}
              </span>
            </div>
            <div className="admin-list-tools">
              <label className="admin-search">
                <Search size={14} />
                <input
                  value={mailboxQuery}
                  onChange={(event) => setMailboxQuery(event.target.value)}
                  placeholder="Search mailboxes"
                  aria-label="Search mailboxes"
                />
              </label>
              <select
                value={mailboxStatusFilter}
                onChange={(event) =>
                  setMailboxStatusFilter(event.target.value as typeof mailboxStatusFilter)
                }
                aria-label="Filter mailboxes by status"
              >
                <option value="all">All statuses</option>
                <option value="active">Active</option>
                <option value="quarantine">Quarantine</option>
                <option value="disabled">Disabled</option>
              </select>
            </div>
            {visibleMailboxes.map((mailbox) => (
              <div className="billing-row" key={mailbox.id}>
                <div>
                  <strong>{mailbox.email}</strong>
                  <small>
                    {roleLabel[mailbox.role]} · {mailbox.displayName} ·{' '}
                    {mailbox.storageUsedGB.toFixed(1)} GB of {mailbox.quotaGB} GB used
                  </small>
                </div>
                <div className="admin-actions">
                  <select
                    className="admin-status-select"
                    value={mailbox.status}
                    aria-label={`Status for ${mailbox.email}`}
                    onChange={(event) => changeStatus(mailbox.id, event.target.value)}
                  >
                    <option value="active">Active</option>
                    <option value="quarantine">Quarantine</option>
                    <option value="disabled">Disabled</option>
                  </select>
                  <input
                    className="admin-quota-input"
                    type="number"
                    min={1}
                    max={100}
                    defaultValue={mailbox.quotaGB}
                    aria-label={`Quota for ${mailbox.email}`}
                    onBlur={(event) => changeQuota(mailbox.id, event.target.value)}
                    title="Storage quota (GB)"
                  />
                  <button
                    type="button"
                    className="secondary-button"
                    onClick={() => resetPassword(mailbox.id)}
                  >
                    <KeyRound size={13} />
                    Reset
                  </button>
                  <button
                    type="button"
                    className="icon-button"
                    aria-label={`Remove ${mailbox.email}`}
                    onClick={() => removeMailbox(mailbox.id)}
                  >
                    <Trash2 size={15} />
                  </button>
                </div>
              </div>
            ))}
            {visibleMailboxes.length === 0 && (
              <p className="settings-hint">No mailboxes match this search.</p>
            )}
          </section>
          <form className="settings-section" onSubmit={addMailbox}>
            <h2>Add a mailbox</h2>
            <label>
              Email address
              <input
                value={mailboxForm.email}
                onChange={(event) =>
                  setMailboxForm((current) => ({ ...current, email: event.target.value }))
                }
                placeholder="sales"
                aria-label="Mailbox address"
              />
            </label>
            <label>
              Display name
              <input
                value={mailboxForm.displayName}
                onChange={(event) =>
                  setMailboxForm((current) => ({ ...current, displayName: event.target.value }))
                }
                placeholder="Sales Team"
                aria-label="Mailbox display name"
              />
            </label>
            <label>
              Storage quota (GB)
              <input
                type="number"
                min={1}
                max={100}
                value={mailboxForm.quotaGB}
                onChange={(event) =>
                  setMailboxForm((current) => ({
                    ...current,
                    quotaGB: Number(event.target.value) || 10,
                  }))
                }
                aria-label="Mailbox quota (GB)"
              />
            </label>
            <div className="row-actions">
              <button type="submit" className="primary-button">
                <Mailbox size={15} />
                Add mailbox
              </button>
            </div>
          </form>
        </>
      )}

      {tab === 'roles' && (
        <>
          <section className="settings-section">
            <div className="admin-section-head">
              <h2>Roles and permissions</h2>
              <span className="admin-section-count">
                {ownersCount} owner · {adminsCount} admins · {membersCount} members
              </span>
            </div>
            <p className="settings-hint">
              <strong>Owner</strong> — full control of the workspace. Always exactly one, and it
              can't be reassigned.
            </p>
            <p className="settings-hint">
              <strong>Admin</strong> — manages mailboxes, aliases, forwarders and security policy.
            </p>
            <p className="settings-hint">
              <strong>Member</strong> — read-only access to the Admin center.
            </p>
            {mailboxes.map((mailbox) => (
              <div className="billing-row" key={mailbox.id}>
                <div>
                  <strong>{mailbox.email}</strong>
                  <small>{mailbox.displayName}</small>
                </div>
                <div className="admin-actions">
                  <select
                    className="admin-status-select"
                    value={mailbox.role}
                    aria-label={`Role for ${mailbox.email}`}
                    disabled={mailbox.role === 'owner'}
                    onChange={(event) => changeRole(mailbox.id, event.target.value)}
                  >
                    <option value="owner">Owner</option>
                    <option value="admin">Admin</option>
                    <option value="member">Member</option>
                  </select>
                </div>
              </div>
            ))}
          </section>
        </>
      )}

      {tab === 'aliases' && (
        <>
          <section className="settings-section">
            <h2>Aliases</h2>
            {aliases.map((alias) => (
              <div className="billing-row" key={alias.id}>
                <div>
                  <strong>{alias.address}</strong>
                  <small>Forwards to {alias.forwardTo}</small>
                </div>
                <button
                  type="button"
                  className="icon-button"
                  aria-label={`Remove ${alias.address}`}
                  onClick={() => removeAlias(alias.id)}
                >
                  <Trash2 size={15} />
                </button>
              </div>
            ))}
            {aliases.length === 0 && (
              <p className="settings-hint">No aliases yet — add one below.</p>
            )}
          </section>
          <form className="settings-section" onSubmit={addAlias}>
            <h2>Add an alias</h2>
            <label>
              Address
              <input
                value={aliasForm.local}
                onChange={(event) =>
                  setAliasForm((current) => ({ ...current, local: event.target.value }))
                }
                placeholder={`name@${domain.domain}`}
                aria-label="Alias address"
              />
            </label>
            <label>
              Deliver to
              <select
                value={aliasForm.forwardTo}
                onChange={(event) =>
                  setAliasForm((current) => ({ ...current, forwardTo: event.target.value }))
                }
                aria-label="Alias target"
              >
                <option value="">Choose a mailbox…</option>
                {mailboxes
                  .filter((mailbox) => mailbox.status === 'active')
                  .map((mailbox) => (
                    <option key={mailbox.id} value={mailbox.email}>
                      {mailbox.email}
                    </option>
                  ))}
              </select>
            </label>
            <div className="row-actions">
              <button type="submit" className="primary-button" disabled={!aliasForm.forwardTo}>
                <AtSign size={15} />
                Add alias
              </button>
            </div>
          </form>
        </>
      )}

      {tab === 'forwarders' && (
        <>
          <section className="settings-section">
            <h2>Forwarders</h2>
            {forwarders.map((forwarder) => (
              <div className="billing-row" key={forwarder.id}>
                <div>
                  <strong>{forwarder.from}</strong>
                  <small>→ {forwarder.to}</small>
                </div>
                <div className="admin-actions">
                  <span className={forwarder.enabled ? 'billing-paid' : ''}>
                    {forwarder.enabled && <Check size={13} />}
                    {forwarder.enabled ? 'Enabled' : 'Paused'}
                  </span>
                  <button
                    type="button"
                    className="secondary-button"
                    onClick={() => toggleForwarder(forwarder.id)}
                  >
                    {forwarder.enabled ? 'Pause' : 'Enable'}
                  </button>
                  <button
                    type="button"
                    className="icon-button"
                    aria-label={`Remove forwarder ${forwarder.from}`}
                    onClick={() => removeForwarder(forwarder.id)}
                  >
                    <Trash2 size={15} />
                  </button>
                </div>
              </div>
            ))}
            {forwarders.length === 0 && (
              <p className="settings-hint">No forwarders yet — add one below.</p>
            )}
          </section>
          <form className="settings-section" onSubmit={addForwarder}>
            <h2>Add a forwarder</h2>
            <label>
              Mailbox
              <select
                value={forwarderForm.from}
                onChange={(event) =>
                  setForwarderForm((current) => ({ ...current, from: event.target.value }))
                }
                aria-label="Forwarder mailbox"
              >
                {mailboxes
                  .filter((mailbox) => mailbox.status === 'active')
                  .map((mailbox) => (
                    <option key={mailbox.id} value={mailbox.email}>
                      {mailbox.email}
                    </option>
                  ))}
              </select>
            </label>
            <label>
              Forward to
              <input
                value={forwarderForm.to}
                onChange={(event) =>
                  setForwarderForm((current) => ({ ...current, to: event.target.value }))
                }
                placeholder="someone@example.com"
                aria-label="Forwarder target"
              />
            </label>
            <div className="row-actions">
              <button type="submit" className="primary-button" disabled={!forwarderForm.from}>
                <RefreshCw size={15} />
                Add forwarder
              </button>
            </div>
          </form>
        </>
      )}

      {tab === 'domain' && (
        <>
          <section className="settings-section">
            <h2>{domain.domain}</h2>
            <div className="billing-plan">
              <div>
                <strong>Domain status</strong>
                <small>
                  {dnsVerified
                    ? 'All records verified — mail is flowing.'
                    : `${Object.values(dns).filter(Boolean).length} of ${dnsRecords.length} records verified`}
                </small>
              </div>
              <button type="button" className="secondary-button" onClick={verifyRecords}>
                <RefreshCw size={14} />
                Check DNS records
              </button>
            </div>
          </section>
          <section className="settings-section">
            <h2>DNS records</h2>
            {dnsRecords.map((record) => (
              <div className="billing-row" key={record.id}>
                <div>
                  <strong>{record.name}</strong>
                  <small className="admin-record">{recordValue(record)}</small>
                </div>
                <span className={`billing-paid ${dns[record.id] ? '' : 'admin-record--pending'}`}>
                  {dns[record.id] && <Check size={13} />}
                  {dns[record.id] ? 'Verified' : 'Required'}
                </span>
                <button
                  type="button"
                  className="secondary-button"
                  onClick={() => void copyRecord({ id: record.id, value: recordValue(record) })}
                >
                  {copied === record.id ? <Check size={14} /> : <Copy size={14} />}
                  {copied === record.id ? 'Copied' : 'Copy'}
                </button>
              </div>
            ))}
          </section>
          <section className="settings-section">
            <h2>Catch-all</h2>
            <label className="settings-options">
              <input
                type="checkbox"
                checked={domain.catchAllEnabled}
                onChange={(event) => setCatchAllEnabled(event.target.checked)}
              />{' '}
              Deliver mail for unknown addresses at {domain.domain}
            </label>
            {domain.catchAllEnabled && (
              <label>
                Deliver to
                <select
                  value={domain.catchAll}
                  onChange={(event) => setCatchAllTarget(event.target.value)}
                  aria-label="Catch-all recipient"
                >
                  {mailboxes
                    .filter((mailbox) => mailbox.status === 'active')
                    .map((mailbox) => (
                      <option key={mailbox.id} value={mailbox.email}>
                        {mailbox.email}
                      </option>
                    ))}
                </select>
              </label>
            )}
          </section>
        </>
      )}

      {tab === 'security' && (
        <>
          <section className="settings-section">
            <h2>Spam and deliverability</h2>
            <label className="admin-range">
              <span>Spam sensitivity — {security.spamThreshold} / 10</span>
              <input
                type="range"
                min={1}
                max={10}
                step={1}
                value={security.spamThreshold}
                aria-label="Spam threshold"
                onChange={(event) =>
                  updateSecurityAndLog(
                    { spamThreshold: Number(event.target.value) },
                    'Spam threshold changed',
                    `${event.target.value} / 10`,
                  )
                }
              />
            </label>
            <div className="settings-options">
              <label>
                <input
                  type="checkbox"
                  checked={security.requireTls}
                  onChange={(event) =>
                    updateSecurityAndLog(
                      { requireTls: event.target.checked },
                      event.target.checked ? 'Inbound TLS enforced' : 'Inbound TLS allowed',
                      domain.domain,
                    )
                  }
                />{' '}
                Require TLS for inbound mail
              </label>
              <label>
                <input
                  type="checkbox"
                  checked={security.scanAttachments}
                  onChange={(event) =>
                    updateSecurityAndLog(
                      { scanAttachments: event.target.checked },
                      event.target.checked
                        ? 'Attachment scanning enabled'
                        : 'Attachment scanning disabled',
                      '',
                    )
                  }
                />{' '}
                Scan attachments for malware
              </label>
            </div>
            <label>
              DMARC policy
              <select
                value={security.dmarcPolicy}
                aria-label="DMARC policy"
                onChange={(event) =>
                  updateSecurityAndLog(
                    { dmarcPolicy: event.target.value as SecuritySettings['dmarcPolicy'] },
                    'DMARC policy changed',
                    event.target.value,
                  )
                }
              >
                <option value="none">No action</option>
                <option value="quarantine">Quarantine</option>
                <option value="reject">Reject</option>
              </select>
            </label>
            <p className="settings-hint">
              DMARC policy is <strong>{dmarcLabel}</strong> — the DNS record above reflects this
              choice.
            </p>
          </section>
          <section className="settings-section">
            <h2>Retention policy</h2>
            <label>
              Purge messages older than
              <select
                value={security.retentionDays}
                aria-label="Retention policy"
                onChange={(event) =>
                  updateSecurityAndLog(
                    { retentionDays: Number(event.target.value) },
                    'Retention policy changed',
                    retentionLabel(Number(event.target.value)),
                  )
                }
              >
                <option value={0}>Keep forever</option>
                <option value={30}>30 days</option>
                <option value={90}>90 days</option>
                <option value={180}>180 days</option>
                <option value={365}>1 year</option>
              </select>
            </label>
            <div className="settings-options">
              <label>
                <input
                  type="checkbox"
                  checked={security.trashAutoPurge}
                  onChange={(event) =>
                    updateSecurityAndLog(
                      { trashAutoPurge: event.target.checked },
                      event.target.checked
                        ? 'Trash auto-purge enabled'
                        : 'Trash auto-purge disabled',
                      '',
                    )
                  }
                />{' '}
                Automatically empty Trash and Spam at the end of the retention window
              </label>
            </div>
            <p className="settings-hint">
              Messages are kept {retentionLabel(security.retentionDays).toLowerCase()}.
              {security.trashAutoPurge ? ' Trash and Spam are purged automatically.' : ''}
            </p>
          </section>
          <section className="settings-section">
            <h2>Single sign-on</h2>
            <div className="settings-options">
              <label>
                <input
                  type="checkbox"
                  checked={sso.enabled}
                  onChange={(event) =>
                    updateSsoAndLog(
                      { enabled: event.target.checked },
                      event.target.checked ? 'SSO enabled' : 'SSO disabled',
                      domain.domain,
                    )
                  }
                />{' '}
                Require single sign-on (SAML) for the {domain.domain} domain
              </label>
            </div>
            {sso.enabled && (
              <>
                <label>
                  Identity provider
                  <select
                    value={sso.provider}
                    aria-label="SSO provider"
                    onChange={(event) =>
                      updateSsoAndLog(
                        { provider: event.target.value as SsoSettings['provider'] },
                        'SSO provider changed',
                        ssoProviders[event.target.value as SsoSettings['provider']],
                      )
                    }
                  >
                    {(Object.keys(ssoProviders) as SsoSettings['provider'][]).map((provider) => (
                      <option key={provider} value={provider}>
                        {ssoProviders[provider]}
                      </option>
                    ))}
                  </select>
                </label>
                <label>
                  SAML entity ID
                  <input
                    value={sso.entityId}
                    aria-label="SAML entity ID"
                    placeholder="https://harbor.co/saml2"
                    onChange={(event) =>
                      setSso((current) => ({ ...current, entityId: event.target.value }))
                    }
                    onBlur={(event) =>
                      updateSsoAndLog(
                        { entityId: event.target.value.trim() },
                        'SAML entity ID updated',
                        event.target.value.trim() || seedSso.entityId,
                      )
                    }
                  />
                </label>
                <div className="settings-options">
                  <label>
                    <input
                      type="checkbox"
                      checked={sso.enforce}
                      onChange={(event) =>
                        updateSsoAndLog(
                          { enforce: event.target.checked },
                          event.target.checked
                            ? 'SSO enforcement enabled'
                            : 'SSO enforcement disabled',
                          '',
                        )
                      }
                    />{' '}
                    Enforce SSO for all mailboxes — password sign-in is disabled
                  </label>
                </div>
                <p className="settings-hint">
                  Consume URL: <strong>https://mail.{domain.domain}/sso/saml2</strong> — the
                  identity provider posts SAML responses to this endpoint.
                </p>
              </>
            )}
            {!sso.enabled && (
              <p className="settings-hint">
                Password sign-in only. Connect an identity provider to enable single sign-on.
              </p>
            )}
          </section>
          <section className="settings-section">
            <h2>Blocked senders</h2>
            {security.blockedSenders.map((sender) => (
              <div className="billing-row" key={sender}>
                <div>
                  <strong>{sender}</strong>
                  <small>Blocked from delivering to any mailbox</small>
                </div>
                <button
                  type="button"
                  className="icon-button"
                  aria-label={`Remove blocked sender ${sender}`}
                  onClick={() => removeBlockedSender(sender)}
                >
                  <Trash2 size={15} />
                </button>
              </div>
            ))}
            {security.blockedSenders.length === 0 && (
              <p className="settings-hint">No senders blocked — add one below.</p>
            )}
            <form className="admin-inline-form" onSubmit={addBlockedSender}>
              <input
                value={blockedForm}
                onChange={(event) => setBlockedForm(event.target.value)}
                placeholder="sender@example.com"
                aria-label="Blocked sender"
              />
              <button type="submit" className="primary-button">
                <ShieldAlert size={15} />
                Block sender
              </button>
            </form>
          </section>
        </>
      )}

      {tab === 'quarantine' && (
        <section className="settings-section">
          <h2>Quarantined messages</h2>
          {quarantine.map((item) => (
            <div className="billing-row" key={item.id}>
              <div>
                <strong>{item.subject}</strong>
                <small>
                  {item.from} → {item.to} · {item.reason} · {item.sizeKB} KB
                </small>
              </div>
              <span className="admin-audit-time">{timeFmt(item.date)}</span>
              <div className="admin-actions">
                <button
                  type="button"
                  className="secondary-button"
                  onClick={() => releaseQuarantine(item.id)}
                >
                  <ShieldCheck size={13} />
                  Release
                </button>
                <button
                  type="button"
                  className="icon-button"
                  aria-label={`Delete quarantined message ${item.subject}`}
                  onClick={() => removeQuarantine(item.id)}
                >
                  <Trash2 size={15} />
                </button>
              </div>
            </div>
          ))}
          {quarantine.length === 0 && (
            <p className="settings-hint">Nothing quarantined right now.</p>
          )}
        </section>
      )}

      {tab === 'audit' && (
        <section className="settings-section">
          <h2>Audit log</h2>
          {audit.map((entry) => (
            <div className="billing-row" key={entry.id}>
              <div>
                <strong>{entry.action}</strong>
                <small>{entry.detail}</small>
              </div>
              <span className="admin-audit-time">{timeFmt(entry.time)}</span>
            </div>
          ))}
          {audit.length === 0 && <p className="settings-hint">No activity recorded yet.</p>}
        </section>
      )}
    </div>
  )
}
