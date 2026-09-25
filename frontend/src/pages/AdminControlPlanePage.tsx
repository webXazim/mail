import { useCallback, useEffect, useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { Activity, Building2, CreditCard, Globe2, Mail, RefreshCw, Save, Search, ShieldAlert, Users } from 'lucide-react'
import {
  platformAdminApi,
  type AdminBusiness,
  type AdminBusinessMember,
  type AdminHostedDomain,
  type AdminHostedMailbox,
  type AdminRecoveryItem,
  type PlatformControls,
} from '../services/admin'

type Tab = 'controls' | 'businesses' | 'domains' | 'mailboxes' | 'recovery'

const PAGE_SIZE = 50
const GB = 1024 ** 3

const defaultControls: PlatformControls = {
  public_signup_enabled: true,
  business_creation_enabled: true,
  plan_ordering_enabled: true,
  domain_onboarding_enabled: true,
  mailbox_provisioning_enabled: true,
  outbound_sending_enabled: true,
  maintenance_message: '',
}

const formatDate = (value?: string | null) => value ? new Date(value).toLocaleString() : '—'
const formatBytes = (value?: number | null) => {
  const bytes = Math.max(0, Number(value ?? 0))
  if (bytes < 1024 ** 2) return `${Math.round(bytes / 1024)} KB`
  if (bytes < 1024 ** 3) return `${(bytes / 1024 ** 2).toFixed(1)} MB`
  return `${(bytes / 1024 ** 3).toFixed(2)} GB`
}

function Pager({ offset, total, onChange }: { offset: number; total: number; onChange: (offset: number) => void }) {
  if (total <= PAGE_SIZE) return null
  const start = Math.min(total, offset + 1)
  const end = Math.min(total, offset + PAGE_SIZE)
  return <div className="admin-pagination" aria-label="Result pages">
    <span>{start}–{end} of {total}</span>
    <div className="admin-actions">
      <button type="button" className="secondary-button" disabled={offset === 0} onClick={() => onChange(Math.max(0, offset - PAGE_SIZE))}>Previous</button>
      <button type="button" className="secondary-button" disabled={offset + PAGE_SIZE >= total} onClick={() => onChange(offset + PAGE_SIZE)}>Next</button>
    </div>
  </div>
}

export function AdminControlPlanePage() {
  const navigate = useNavigate()
  const [tab, setTab] = useState<Tab>('controls')
  const [notice, setNotice] = useState('')
  const [error, setError] = useState('')
  const [loading, setLoading] = useState(false)
  const [controls, setControls] = useState<PlatformControls>(defaultControls)
  const [savedControls, setSavedControls] = useState<PlatformControls>(defaultControls)
  const [searchDraft, setSearchDraft] = useState('')
  const [query, setQuery] = useState('')
  const [status, setStatus] = useState('')
  const [offset, setOffset] = useState(0)
  const [businesses, setBusinesses] = useState<AdminBusiness[]>([])
  const [businessTotal, setBusinessTotal] = useState(0)
  const [domains, setDomains] = useState<AdminHostedDomain[]>([])
  const [domainTotal, setDomainTotal] = useState(0)
  const [mailboxes, setMailboxes] = useState<AdminHostedMailbox[]>([])
  const [mailboxTotal, setMailboxTotal] = useState(0)
  const [recovery, setRecovery] = useState<AdminRecoveryItem[]>([])
  const [selectedBusiness, setSelectedBusiness] = useState<AdminBusiness | null>(null)
  const [members, setMembers] = useState<AdminBusinessMember[]>([])

  const show = (message: string) => { setNotice(message); setError('') }
  const fail = (reason: unknown) => { setError(reason instanceof Error ? reason.message : 'Operation failed'); setNotice('') }

  const loadControls = useCallback(async () => {
    const next = await platformAdminApi.controls()
    setControls(next)
    setSavedControls(next)
  }, [])
  const loadBusinesses = useCallback(async () => {
    const data = await platformAdminApi.businesses({ q: query, status: status || undefined, limit: PAGE_SIZE, offset })
    setBusinesses(data.businesses); setBusinessTotal(data.total)
  }, [query, status, offset])
  const loadDomains = useCallback(async () => {
    const data = await platformAdminApi.domains({ q: query, status: status || undefined, limit: PAGE_SIZE, offset })
    setDomains(data.domains); setDomainTotal(data.total)
  }, [query, status, offset])
  const loadMailboxes = useCallback(async () => {
    const data = await platformAdminApi.mailboxes({ q: query, status: status || undefined, limit: PAGE_SIZE, offset })
    setMailboxes(data.mailboxes); setMailboxTotal(data.total)
  }, [query, status, offset])
  const loadRecovery = useCallback(async () => {
    const data = await platformAdminApi.recovery()
    setRecovery([...data.provisioning, ...data.imports, ...data.scheduled, ...data.billing_email].sort((a, b) => b.updated_at.localeCompare(a.updated_at)))
  }, [])

  const reload = useCallback(async () => {
    setLoading(true)
    try {
      if (tab === 'controls') await loadControls()
      if (tab === 'businesses') await loadBusinesses()
      if (tab === 'domains') await loadDomains()
      if (tab === 'mailboxes') await loadMailboxes()
      if (tab === 'recovery') await loadRecovery()
    } catch (reason) { fail(reason) } finally { setLoading(false) }
  }, [tab, loadControls, loadBusinesses, loadDomains, loadMailboxes, loadRecovery])

  useEffect(() => { void reload() }, [reload])

  const changeTab = (next: Tab) => {
    setTab(next)
    setSearchDraft('')
    setQuery('')
    setStatus('')
    setOffset(0)
    setSelectedBusiness(null)
    setMembers([])
  }

  const openMembers = async (business: AdminBusiness) => {
    setSelectedBusiness(business)
    try { setMembers((await platformAdminApi.businessMembers(business.id)).members) } catch (reason) { fail(reason) }
  }

  const saveControls = async () => {
    const switches: (keyof PlatformControls)[] = [
      'public_signup_enabled', 'business_creation_enabled', 'plan_ordering_enabled',
      'domain_onboarding_enabled', 'mailbox_provisioning_enabled', 'outbound_sending_enabled',
    ]
    const newlyPaused = switches.filter((key) => savedControls[key] === true && controls[key] === false)
    if (newlyPaused.length > 0 && !window.confirm(`Pause ${newlyPaused.length} production service function${newlyPaused.length === 1 ? '' : 's'} now? Existing mailboxes remain intact.`)) return
    setLoading(true)
    try {
      const next = await platformAdminApi.updateControls(controls)
      setControls(next); setSavedControls(next); show('Platform runtime controls saved.')
    } catch (reason) { fail(reason) } finally { setLoading(false) }
  }

  const updateBusiness = async (business: AdminBusiness, next: 'active' | 'suspended' | 'closed') => {
    const reason = next === 'active' ? '' : window.prompt(`Reason for ${next} status`, business.status_reason || '') ?? ''
    if (next !== 'active' && !reason.trim()) return
    const confirmName = next === 'closed' ? window.prompt(`Type ${business.name} to confirm closure. Its subscription must already be cancelled.`) ?? '' : ''
    if (next === 'closed' && confirmName !== business.name) return
    try { await platformAdminApi.updateBusinessStatus(business.id, { status: next, reason, confirm_name: confirmName }); await loadBusinesses(); show(`Business is now ${next}.`) } catch (reasonValue) { fail(reasonValue) }
  }

  const updateMember = async (member: AdminBusinessMember, patch: Partial<Pick<AdminBusinessMember, 'role' | 'status'>>) => {
    if (!selectedBusiness) return
    try {
      await platformAdminApi.updateBusinessMember(selectedBusiness.id, member.user_id, { role: patch.role ?? member.role, status: patch.status ?? member.status })
      setMembers((await platformAdminApi.businessMembers(selectedBusiness.id)).members)
      show('Business membership updated and provider access reconciliation queued.')
    } catch (reason) { fail(reason) }
  }

  const removeMember = async (member: AdminBusinessMember) => {
    if (!selectedBusiness || !window.confirm(`Remove ${member.email} from ${selectedBusiness.name}? Hosted mailboxes must be reassigned or deleted first.`)) return
    try { await platformAdminApi.removeBusinessMember(selectedBusiness.id, member.user_id); setMembers((await platformAdminApi.businessMembers(selectedBusiness.id)).members); show('Business member removed.') } catch (reason) { fail(reason) }
  }

  const actDomain = async (domain: AdminHostedDomain, action: 'suspend' | 'resume' | 'check_dns' | 'provision' | 'delete') => {
    let confirmDomain = ''
    if (action === 'suspend' && !window.confirm(`Suspend hosted-mail access for ${domain.domain}?`)) return
    if (action === 'delete') {
      confirmDomain = window.prompt(`Type ${domain.domain} to release this domain. All mailboxes, aliases and groups must already be removed.`) ?? ''
      if (confirmDomain !== domain.domain) return
    }
    try { await platformAdminApi.domainAction(domain.id, action, confirmDomain); await loadDomains(); show(`Domain action '${action}' completed.`) } catch (reason) { fail(reason) }
  }

  const actMailbox = async (mailbox: AdminHostedMailbox, action: 'suspend' | 'activate' | 'delete') => {
    let confirmAddress = ''
    if (action === 'delete') {
      confirmAddress = window.prompt(`Type ${mailbox.address} to permanently remove this hosted mailbox`) ?? ''
      if (confirmAddress !== mailbox.address) return
    }
    try { await platformAdminApi.mailboxAction(mailbox.id, action, { confirmAddress }); await loadMailboxes(); show(`Mailbox action '${action}' queued.`) } catch (reason) { fail(reason) }
  }

  const changeMailboxStorage = async (mailbox: AdminHostedMailbox) => {
    const currentGb = Math.max(0, mailbox.quota_bytes / GB).toFixed(2)
    const raw = window.prompt(`Storage allocation for ${mailbox.address} in GB. Enter "default" to restore the plan default.`, currentGb)
    if (raw === null) return
    const normalized = raw.trim().toLowerCase()
    try {
      if (normalized === 'default') {
        await platformAdminApi.mailboxAction(mailbox.id, 'set_quota', { resetToDefault: true })
      } else {
        const gb = Number(normalized)
        if (!Number.isFinite(gb) || gb <= 0) { fail(new Error('Enter a positive storage size in GB, or "default".')); return }
        await platformAdminApi.mailboxAction(mailbox.id, 'set_quota', { quotaBytes: Math.round(gb * GB) })
      }
      await loadMailboxes(); show('Mailbox storage allocation updated and provider quota sync queued.')
    } catch (reason) { fail(reason) }
  }

  const retryJob = async (job: AdminRecoveryItem) => {
    try { await platformAdminApi.retryRecovery(job.id, job.kind); await loadRecovery(); show('Recovery job rescheduled.') } catch (reason) { fail(reason) }
  }

  const statusOptions = tab === 'businesses'
    ? ['', 'active', 'suspended', 'closed']
    : tab === 'domains'
      ? ['', 'pending_verification', 'verified', 'provisioning', 'dns_pending', 'active', 'degraded', 'suspended', 'failed']
      : tab === 'mailboxes'
        ? ['', 'pending', 'active', 'suspended', 'deleting', 'error']
        : ['']

  return (
    <div className="admin-page" role="region" aria-label="SaaS control plane">
      <header className="calendar-head">
        <div><p className="eyebrow">Platform admin / Operations</p><h1>SaaS control plane</h1></div>
        <div className="calendar-head__actions">
          <button className="secondary-button" type="button" onClick={() => navigate('/mail/admin/operations')}><ShieldAlert size={14}/>Platform operations</button>
          <button className="secondary-button" type="button" onClick={() => navigate('/mail/admin/billing')}><CreditCard size={14}/>Payments &amp; plans</button>
        </div>
      </header>

      <nav className="admin-nav" aria-label="Control plane sections">
        <button type="button" className={tab === 'controls' ? 'admin-nav--active' : ''} onClick={() => changeTab('controls')}><ShieldAlert size={14}/>Runtime controls</button>
        <button type="button" className={tab === 'businesses' ? 'admin-nav--active' : ''} onClick={() => changeTab('businesses')}><Building2 size={14}/>Businesses</button>
        <button type="button" className={tab === 'domains' ? 'admin-nav--active' : ''} onClick={() => changeTab('domains')}><Globe2 size={14}/>Hosted domains</button>
        <button type="button" className={tab === 'mailboxes' ? 'admin-nav--active' : ''} onClick={() => changeTab('mailboxes')}><Mail size={14}/>Hosted mailboxes</button>
        <button type="button" className={tab === 'recovery' ? 'admin-nav--active' : ''} onClick={() => changeTab('recovery')}><Activity size={14}/>Recovery</button>
      </nav>

      {notice && <p className="settings-notice settings-notice--ok">{notice}</p>}
      {error && <p className="settings-notice settings-notice--error">{error}</p>}

      {tab === 'controls' && <section className="settings-section">
        <div className="admin-section-head"><div><h2>Emergency service controls</h2><small>Database-backed switches enforced by every API replica. Transactional account/billing email remains available when customer outbound mail is paused.</small></div></div>
        <p className="settings-hint">For first public launch, keep every switch closed until <strong>Launch freeze</strong> passes. Then open in order: public signup → business creation → plan ordering → domain onboarding → mailbox provisioning → customer outbound sending, verifying alerts and one real customer flow between stages.</p>
        <div className="admin-toggle-grid">
          {([
            ['public_signup_enabled', 'Public signup', 'Allow new CS Mail platform accounts.'],
            ['business_creation_enabled', 'Business creation', 'Allow customers to create new business workspaces.'],
            ['plan_ordering_enabled', 'Plan ordering', 'Allow new plan orders and invoices.'],
            ['domain_onboarding_enabled', 'Domain onboarding', 'Allow domain claim, verification and provider provisioning.'],
            ['mailbox_provisioning_enabled', 'Mailbox provisioning', 'Allow creation/acceptance of new hosted mailboxes.'],
            ['outbound_sending_enabled', 'Customer outbound sending', 'Allow direct and scheduled outgoing customer mail.'],
          ] as const).map(([key, label, help]) => <label className="admin-control-toggle" key={key}>
            <span><strong>{label}</strong><small>{help}</small></span>
            <input type="checkbox" checked={controls[key]} onChange={(event) => setControls((current) => ({ ...current, [key]: event.target.checked }))}/>
          </label>)}
        </div>
        <label className="settings-field"><span>Maintenance message</span><textarea rows={3} value={controls.maintenance_message} onChange={(event) => setControls((current) => ({ ...current, maintenance_message: event.target.value }))} placeholder="Optional public message returned when a paused operation is attempted."/></label>
        <p className="settings-hint">Last changed {formatDate(controls.updated_at)}{controls.updated_by_email ? ` by ${controls.updated_by_email}` : ''}. Every change is also written to the immutable audit log.</p>
        <div className="admin-actions"><button type="button" className="primary-button" onClick={() => void saveControls()} disabled={loading}><Save size={14}/>Save runtime controls</button></div>
      </section>}

      {tab !== 'controls' && tab !== 'recovery' && <section className="settings-section admin-resource-tools">
        <form className="admin-search" onSubmit={(event) => { event.preventDefault(); setOffset(0); setQuery(searchDraft.trim()) }}>
          <Search size={14}/><input value={searchDraft} onChange={(event) => setSearchDraft(event.target.value)} placeholder={`Search ${tab}`}/><button type="submit" className="secondary-button">Search</button>
        </form>
        <select value={status} onChange={(event) => { setStatus(event.target.value); setOffset(0) }}>{statusOptions.map((item) => <option value={item} key={item || 'all'}>{item ? item.replaceAll('_', ' ') : 'All statuses'}</option>)}</select>
        <button type="button" className="secondary-button" onClick={() => void reload()}><RefreshCw size={14}/>Refresh</button>
      </section>}

      {tab === 'businesses' && <>
        <section className="settings-section"><div className="admin-section-head"><h2>Customer businesses</h2><span className="admin-section-count">{businessTotal} total</span></div>
          <div className="admin-resource-list">{businesses.map((business) => <div className="admin-resource-row" key={business.id}>
            <div className="admin-resource-main"><strong>{business.name}</strong><small>{business.owner_email ?? 'No active owner'} · {business.plan_name ?? 'No plan'} · {business.purchased_mailbox_count ?? 0} mailboxes</small><small>{business.member_count} members · {business.domain_count} domains · {business.mailbox_count} hosted mailboxes · Expires {formatDate(business.current_period_end)}</small><small>Subscription {business.subscription_status ?? 'none'} · Activation {business.assignment_source?.replaceAll('_', ' ') ?? '—'}</small>{business.status_reason && <small>{business.status_reason}</small>}</div>
            <span className={`status-pill status-pill--${business.status}`}>{business.status}</span>
            <div className="admin-actions"><button className="secondary-button" type="button" onClick={() => void openMembers(business)}><Users size={14}/>Members</button><button className="secondary-button" type="button" onClick={() => navigate(`/mail/admin/billing?business=${business.id}`)}>Plan</button>{business.status === 'active' ? <button className="secondary-button" type="button" onClick={() => void updateBusiness(business, 'suspended')}>Suspend</button> : business.status !== 'closed' && <button className="secondary-button" type="button" onClick={() => void updateBusiness(business, 'active')}>Reactivate</button>}{business.status !== 'closed' && <button className="secondary-button admin-danger" type="button" onClick={() => void updateBusiness(business, 'closed')}>Close</button>}</div>
          </div>)}</div>
          <Pager offset={offset} total={businessTotal} onChange={setOffset}/>
        </section>
        {selectedBusiness && <section className="settings-section"><div className="admin-section-head"><div><h2>{selectedBusiness.name} members</h2><small>Business roles are separate from CS Mail platform roles. Suspending membership also reconciles provider mailbox login access.</small></div></div>
          {members.map((member) => <div className="billing-row" key={member.user_id}><div><strong>{member.display_name || member.email}</strong><small>{member.email} · Platform: {member.platform_role.replaceAll('_', ' ')}</small></div><div className="admin-actions"><select value={member.role} onChange={(event) => void updateMember(member, { role: event.target.value as AdminBusinessMember['role'] })}><option value="owner">Owner</option><option value="admin">Admin</option><option value="billing">Billing</option><option value="member">Member</option></select><select value={member.status} onChange={(event) => void updateMember(member, { status: event.target.value as AdminBusinessMember['status'] })}><option value="active">Active</option><option value="suspended">Suspended</option></select><button type="button" className="secondary-button admin-danger" onClick={() => void removeMember(member)}>Remove</button></div></div>)}
        </section>}
      </>}

      {tab === 'domains' && <section className="settings-section"><div className="admin-section-head"><h2>Hosted domains</h2><span className="admin-section-count">{domainTotal} total</span></div>
        <div className="admin-resource-list">{domains.map((domain) => <div className="admin-resource-row" key={domain.id}><div className="admin-resource-main"><strong>{domain.domain}</strong><small>{domain.organization_name} · {domain.mailbox_count} mailboxes · DNS {domain.dns_ready ? 'ready' : 'not ready'}</small><small>MX {domain.dns.mx ? '✓' : '—'} · SPF {domain.dns.spf ? '✓' : '—'} · DKIM {domain.dns.dkim ? '✓' : '—'} · DMARC {domain.dns.dmarc ? '✓' : '—'} · Last check {formatDate(domain.last_dns_readiness_check)}</small>{domain.last_error && <small>{domain.last_error}</small>}</div><span className={`status-pill status-pill--${domain.status}`}>{domain.status.replaceAll('_', ' ')}</span><div className="admin-actions"><button type="button" className="secondary-button" onClick={() => void actDomain(domain, 'check_dns')}>Check DNS</button>{!domain.provider_domain_id && domain.verified_at && <button type="button" className="secondary-button" onClick={() => void actDomain(domain, 'provision')}>Provision</button>}{domain.status === 'suspended' ? <button type="button" className="secondary-button" onClick={() => void actDomain(domain, 'resume')}>Resume</button> : <button type="button" className="secondary-button" onClick={() => void actDomain(domain, 'suspend')}>Suspend</button>}<button type="button" className="secondary-button admin-danger" onClick={() => void actDomain(domain, 'delete')}>Release</button></div></div>)}</div>
        <Pager offset={offset} total={domainTotal} onChange={setOffset}/>
      </section>}

      {tab === 'mailboxes' && <section className="settings-section"><div className="admin-section-head"><h2>Hosted mailboxes</h2><span className="admin-section-count">{mailboxTotal} total</span></div>
        <div className="admin-resource-list">{mailboxes.map((mailbox) => <div className="admin-resource-row" key={mailbox.id}><div className="admin-resource-main"><strong>{mailbox.address}</strong><small>{mailbox.organization_name} · {mailbox.user_email ?? 'Unassigned'} · {formatBytes(mailbox.quota_used)} / {formatBytes(mailbox.quota_bytes)}</small><progress className="admin-storage-progress" max={Math.max(1, mailbox.quota_bytes)} value={Math.min(Math.max(0, mailbox.quota_used ?? 0), Math.max(1, mailbox.quota_bytes))}/><small>Provider sync: {mailbox.sync_status}{mailbox.sync_error ? ` · ${mailbox.sync_error}` : ''}</small></div><span className={`status-pill status-pill--${mailbox.status}`}>{mailbox.status}</span><div className="admin-actions"><button type="button" className="secondary-button" disabled={mailbox.status === 'deleting'} onClick={() => void changeMailboxStorage(mailbox)}>Storage</button>{mailbox.status === 'suspended' ? <button type="button" className="secondary-button" onClick={() => void actMailbox(mailbox, 'activate')}>Activate</button> : mailbox.status !== 'deleting' && <button type="button" className="secondary-button" onClick={() => void actMailbox(mailbox, 'suspend')}>Suspend</button>}<button type="button" className="secondary-button admin-danger" disabled={mailbox.status === 'deleting'} onClick={() => void actMailbox(mailbox, 'delete')}>Delete</button></div></div>)}</div>
        <Pager offset={offset} total={mailboxTotal} onChange={setOffset}/>
      </section>}

      {tab === 'recovery' && <section className="settings-section"><div className="admin-section-head"><div><h2>Durable job recovery</h2><small>Provisioning, mailbox imports, scheduled sends and billing-email outbox.</small></div><button type="button" className="secondary-button" onClick={() => void loadRecovery()}><RefreshCw size={14}/>Refresh</button></div>
        {recovery.length === 0 ? <p className="settings-empty">No retry, failed or in-progress recovery jobs.</p> : recovery.map((job) => { const retryable = (job.kind === 'provisioning' && ['dead', 'retry'].includes(job.status)) || (job.kind === 'import' && job.status === 'failed') || (job.kind === 'scheduled' && job.status === 'dead') || (job.kind === 'billing_email' && job.status === 'failed'); return <div className="billing-row" key={`${job.kind}:${job.id}`}><div><strong>{job.kind.replaceAll('_', ' ')} · {job.target}</strong><small>{job.operation ? `${job.operation} · ` : ''}{job.status} · attempt {job.attempts}{job.max_attempts ? `/${job.max_attempts}` : ''} · {formatDate(job.updated_at)}</small>{job.last_error && <small>{job.last_error}</small>}</div>{retryable && <button type="button" className="secondary-button" onClick={() => void retryJob(job)}><RefreshCw size={14}/>Retry</button>}</div> })}
      </section>}
    </div>
  )
}
