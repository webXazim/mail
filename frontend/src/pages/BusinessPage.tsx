import { useEffect, useMemo, useState, type FormEvent } from 'react'
import { Link } from 'react-router-dom'
import { Building2, Check, Copy, Globe2, Mail, Plus, RefreshCw, ShieldCheck, Trash2, UserPlus, Users } from 'lucide-react'
import { organizationsApi, type BusinessAddress, type OrganizationDetail, type OrganizationInvitation, type OrganizationMember, type OrganizationRole, type OrganizationSummary } from '../services/organizations'
import { profileApi, useProfile } from '../services/profile'

const GIB = 1024 ** 3
const formatStorage = (bytes: number | null | undefined) => {
  if (bytes == null) return '—'
  if (bytes >= GIB) {
    const value = bytes / GIB
    return `${value >= 10 ? value.toFixed(0) : value.toFixed(1)} GB`
  }
  return `${Math.max(0, bytes / 1024 ** 2).toFixed(0)} MB`
}
const quotaDraft = (bytes: number | null | undefined) => bytes == null ? '' : (bytes / GIB).toFixed(bytes % GIB === 0 ? 0 : 1)

export function BusinessPage() {
  const profile = useProfile()
  const [organizations, setOrganizations] = useState<OrganizationSummary[]>([])
  const [activeId, setActiveId] = useState<string | null>(null)
  const [activeMailboxId, setActiveMailboxId] = useState<string | null>(null)
  const [detail, setDetail] = useState<OrganizationDetail | null>(null)
  const [name, setName] = useState('')
  const [error, setError] = useState('')
  const [busy, setBusy] = useState(false)
  const [members, setMembers] = useState<OrganizationMember[]>([])
  const [invitations, setInvitations] = useState<OrganizationInvitation[]>([])
  const [inviteEmail, setInviteEmail] = useState('')
  const [inviteRole, setInviteRole] = useState<OrganizationRole>('member')
  const [domainName, setDomainName] = useState('')
  const [domainNotice, setDomainNotice] = useState('')
  const [mailboxLocal, setMailboxLocal] = useState('')
  const [mailboxDomainId, setMailboxDomainId] = useState('')
  const [mailboxMemberId, setMailboxMemberId] = useState('')
  const [mailboxInviteEmail, setMailboxInviteEmail] = useState('')
  const [mailboxQuotaDrafts, setMailboxQuotaDrafts] = useState<Record<string, string>>({})
  const [addresses, setAddresses] = useState<BusinessAddress[]>([])
  const [addressLocal, setAddressLocal] = useState('')
  const [addressKind, setAddressKind] = useState<'alias' | 'group'>('alias')
  const [addressDomainId, setAddressDomainId] = useState('')
  const [addressMailboxIds, setAddressMailboxIds] = useState<string[]>([])

  const active = useMemo(
    () => organizations.find((item) => item.id === activeId) ?? organizations[0] ?? null,
    [organizations, activeId],
  )

  const load = async (preferred?: string) => {
    const list = await organizationsApi.list()
    setOrganizations(list.organizations)
    const next = preferred || list.active_organization_id || list.organizations[0]?.id || null
    setActiveId(next)
    setActiveMailboxId(list.active_mailbox_id)
    if (next) {
      const nextDetail = await organizationsApi.get(next)
      const domainResult = await organizationsApi.domains(next)
      const mailboxResult = await organizationsApi.mailboxes(next)
      setDetail({ ...nextDetail, domains: domainResult.domains, mailboxes: mailboxResult.mailboxes, storage: mailboxResult.storage })
      setMailboxQuotaDrafts(Object.fromEntries(mailboxResult.mailboxes.map((mailbox) => [mailbox.id, quotaDraft(mailbox.quota_bytes)])))
      const memberResult = await organizationsApi.members(next)
      setMembers(memberResult.members)
      const addressResult = await organizationsApi.addresses(next)
      setAddresses(addressResult.addresses)
      const activeDomain = domainResult.domains.find((item) => item.status === 'active')
      if (activeDomain) {
        setMailboxDomainId((current) => current || activeDomain.id)
        setAddressDomainId((current) => current || activeDomain.id)
      }
      if (nextDetail.role === 'owner' || nextDetail.role === 'admin') {
        const inviteResult = await organizationsApi.invitations(next)
        setInvitations(inviteResult.invitations)
      } else {
        setInvitations([])
      }
    } else {
      setDetail(null)
      setMembers([])
      setInvitations([])
      setAddresses([])
    }
  }

  useEffect(() => {
    load().catch((cause) => setError(cause instanceof Error ? cause.message : 'Unable to load businesses'))
  }, [])

  useEffect(() => {
    const onRealtime = (event: Event) => {
      const detail = (event as CustomEvent<{ kind?: string; payload?: { resource?: string; organization_id?: string } }>).detail
      if (detail?.kind !== 'resource-changed' || detail.payload?.resource !== 'business_domains') return
      if (activeId && detail.payload.organization_id && detail.payload.organization_id !== activeId) return
      load(activeId || undefined).catch(() => undefined)
    }
    window.addEventListener('cs-mail-realtime', onRealtime)
    return () => window.removeEventListener('cs-mail-realtime', onRealtime)
  }, [activeId])

  const createBusiness = async (event: FormEvent) => {
    event.preventDefault()
    if (!name.trim()) return
    setBusy(true)
    setError('')
    try {
      const created = await organizationsApi.create(name.trim())
      await organizationsApi.activate(created.id)
      setName('')
      await profileApi.refresh()
      await load(created.id)
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : 'Unable to create business')
    } finally {
      setBusy(false)
    }
  }

  const switchBusiness = async (id: string) => {
    setBusy(true)
    setError('')
    try {
      await organizationsApi.activate(id)
      await profileApi.refresh()
      await load(id)
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : 'Unable to switch business')
    } finally {
      setBusy(false)
    }
  }

  const inviteMember = async (event: FormEvent) => {
    event.preventDefault()
    if (!detail || !inviteEmail.trim()) return
    setBusy(true)
    setError('')
    try {
      await organizationsApi.invite(detail.id, inviteEmail.trim(), inviteRole)
      setInviteEmail('')
      const result = await organizationsApi.invitations(detail.id)
      setInvitations(result.invitations)
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : 'Unable to send invitation')
    } finally {
      setBusy(false)
    }
  }

  const claimDomain = async (event: FormEvent) => {
    event.preventDefault()
    if (!detail || !domainName.trim()) return
    setBusy(true)
    setError('')
    setDomainNotice('')
    try {
      await organizationsApi.claimDomain(detail.id, domainName.trim())
      setDomainName('')
      setDomainNotice('Domain claim created. Publish the TXT record shown below, then verify ownership.')
      await load(detail.id)
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : 'Unable to claim domain')
    } finally {
      setBusy(false)
    }
  }

  const verifyDomain = async (domainId: string) => {
    if (!detail) return
    setBusy(true)
    setError('')
    setDomainNotice('')
    try {
      const result = await organizationsApi.verifyDomain(detail.id, domainId)
      setDomainNotice(result.message)
      await load(detail.id)
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : 'Unable to verify domain')
    } finally {
      setBusy(false)
    }
  }

  const provisionDomain = async (domainId: string) => {
    if (!detail) return
    setBusy(true)
    setError('')
    setDomainNotice('')
    try {
      const result = await organizationsApi.provisionDomain(detail.id, domainId)
      setDomainNotice(result.message)
      await load(detail.id)
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : 'Unable to provision mail domain')
    } finally {
      setBusy(false)
    }
  }

  const checkDomainDns = async (domainId: string) => {
    if (!detail) return
    setBusy(true)
    setError('')
    setDomainNotice('')
    try {
      const result = await organizationsApi.checkDomainDns(detail.id, domainId)
      setDomainNotice(result.message)
      await load(detail.id)
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : 'Unable to check mail DNS')
    } finally {
      setBusy(false)
    }
  }

  const rotateDomain = async (domainId: string) => {
    if (!detail) return
    setBusy(true)
    setError('')
    try {
      await organizationsApi.rotateDomainChallenge(detail.id, domainId)
      setDomainNotice('A new TXT verification token was generated. Replace the old record before verifying again.')
      await load(detail.id)
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : 'Unable to rotate verification token')
    } finally {
      setBusy(false)
    }
  }

  const releaseDomain = async (domainId: string) => {
    if (!detail) return
    if (!window.confirm('Release this domain claim from this business?')) return
    setBusy(true)
    setError('')
    try {
      await organizationsApi.releaseDomain(detail.id, domainId)
      setDomainNotice('Domain claim released.')
      await load(detail.id)
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : 'Unable to release domain')
    } finally {
      setBusy(false)
    }
  }

  const copyText = async (value: string | null | undefined) => {
    if (!value) return
    try {
      await navigator.clipboard.writeText(value)
      setDomainNotice('Copied to clipboard.')
    } catch {
      setDomainNotice('Copy failed. Select the DNS value manually.')
    }
  }

  const revokeInvitation = async (invitationId: string) => {
    if (!detail) return
    setBusy(true)
    setError('')
    try {
      await organizationsApi.revokeInvitation(detail.id, invitationId)
      const result = await organizationsApi.invitations(detail.id)
      setInvitations(result.invitations)
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : 'Unable to revoke invitation')
    } finally {
      setBusy(false)
    }
  }


  const createMailbox = async (event: FormEvent) => {
    event.preventDefault()
    if (!detail || !mailboxLocal.trim() || !mailboxDomainId) return
    if (!mailboxMemberId && !mailboxInviteEmail.trim()) {
      setError('Choose an existing member or enter an invitation email.')
      return
    }
    setBusy(true); setError('')
    try {
      await organizationsApi.createMailbox(detail.id, {
        domain_id: mailboxDomainId,
        local_part: mailboxLocal.trim(),
        member_user_id: mailboxMemberId || undefined,
        invite_email: mailboxMemberId ? undefined : mailboxInviteEmail.trim(),
        role: 'member',
      })
      setMailboxLocal(''); setMailboxInviteEmail('')
      await load(detail.id)
    } catch (cause) { setError(cause instanceof Error ? cause.message : 'Unable to create mailbox') }
    finally { setBusy(false) }
  }

  const switchMailbox = async (mailboxId: string) => {
    if (!detail) return
    setBusy(true); setError('')
    try {
      await organizationsApi.activateMailbox(detail.id, mailboxId)
      setActiveMailboxId(mailboxId)
      await profileApi.refresh()
      window.dispatchEvent(new Event('cs-mail-folders-changed'))
    } catch (cause) { setError(cause instanceof Error ? cause.message : 'Unable to switch mailbox') }
    finally { setBusy(false) }
  }

  const setMailboxStatus = async (mailboxId: string, status: 'active' | 'suspended') => {
    if (!detail) return
    setBusy(true); setError('')
    try {
      await organizationsApi.updateMailbox(detail.id, mailboxId, status)
      await profileApi.refresh()
      await load(detail.id)
      window.dispatchEvent(new Event('cs-mail-folders-changed'))
    }
    catch (cause) { setError(cause instanceof Error ? cause.message : 'Unable to update mailbox') }
    finally { setBusy(false) }
  }

  const setMailboxStorage = async (mailboxId: string, resetToDefault = false) => {
    if (!detail) return
    const raw = mailboxQuotaDrafts[mailboxId] ?? ''
    const gb = Number(raw)
    if (!resetToDefault && (!Number.isFinite(gb) || gb <= 0)) {
      setError('Enter a valid mailbox storage size in GB.')
      return
    }
    setBusy(true); setError('')
    try {
      await organizationsApi.updateMailboxStorage(detail.id, mailboxId, resetToDefault ? undefined : Math.round(gb * GIB), resetToDefault)
      await profileApi.refresh()
      await load(detail.id)
    } catch (cause) { setError(cause instanceof Error ? cause.message : 'Unable to update mailbox storage') }
    finally { setBusy(false) }
  }

  const removeMailbox = async (mailboxId: string) => {
    if (!detail || !window.confirm('Delete this hosted mailbox? Mail-server deletion is queued safely.')) return
    setBusy(true); setError('')
    try {
      await organizationsApi.deleteMailbox(detail.id, mailboxId)
      await profileApi.refresh()
      await load(detail.id)
      window.dispatchEvent(new Event('cs-mail-folders-changed'))
    }
    catch (cause) { setError(cause instanceof Error ? cause.message : 'Unable to delete mailbox') }
    finally { setBusy(false) }
  }

  const createBusinessAddress = async (event: FormEvent) => {
    event.preventDefault()
    if (!detail || !addressLocal.trim() || !addressDomainId || addressMailboxIds.length === 0) return
    setBusy(true); setError('')
    try {
      await organizationsApi.createAddress(detail.id, { domain_id: addressDomainId, local_part: addressLocal.trim(), kind: addressKind, mailbox_ids: addressMailboxIds })
      setAddressLocal(''); setAddressMailboxIds([]); await load(detail.id)
    } catch (cause) { setError(cause instanceof Error ? cause.message : 'Unable to create alias/group') }
    finally { setBusy(false) }
  }

  const removeBusinessAddress = async (addressId: string) => {
    if (!detail) return
    setBusy(true); setError('')
    try { await organizationsApi.deleteAddress(detail.id, addressId); await load(detail.id) }
    catch (cause) { setError(cause instanceof Error ? cause.message : 'Unable to delete alias/group') }
    finally { setBusy(false) }
  }

  return (
    <section className="business-page">
      <header className="page-head">
        <div>
          <p className="eyebrow">Business workspace</p>
          <h1>Business admin</h1>
          <p>Organizations own domains and mailboxes. Your CS Mail login stays separate from hosted email addresses.</p>
        </div>
      </header>

      <section className="business-setup" aria-label="Business mail setup">
        <h2>Set up business mail</h2>
        <p>Your login is separate from your company mailbox. Mail becomes available after the domain and an address are ready.</p>
        <ol>
          <li className={active ? 'business-setup__done' : ''}>
            <strong>1. Business billing identity</strong>
            <span>{active ? active.name : 'Enter your company name below.'}</span>
          </li>
          <li className="business-setup__done">
            <strong>2. Review your plan</strong>
            <Link to="/mail/billing">View active plan and billing</Link>
          </li>
          <li className={detail?.domains.some((domain) => domain.status === 'active') ? 'business-setup__done' : ''}>
            <strong>3. Verify your domain and mail DNS</strong>
            <span>{!detail ? 'Create a business first.' : detail.domains.some((domain) => domain.status === 'active') ? 'Domain ready' : 'Add your domain below, prove ownership, then publish the mail records.'}</span>
          </li>
          <li className={profile?.has_mailbox ? 'business-setup__done' : ''}>
            <strong>4. Create and use your mailbox</strong>
            <span>{profile?.has_mailbox ? profile.mailbox_email : 'An active domain is required before you can create an address.'}</span>
          </li>
        </ol>
      </section>

      {error && <p className="business-alert business-alert--error">{error}</p>}

      <div className="business-layout">
        <aside className="business-list" aria-label="Businesses">
          {organizations.map((organization) => (
            <button
              type="button"
              key={organization.id}
              className={`business-list__item ${active?.id === organization.id ? 'business-list__item--active' : ''}`}
              onClick={() => void switchBusiness(organization.id)}
              disabled={busy}
            >
              <span className="business-list__icon"><Building2 size={17} /></span>
              <span>
                <strong>{organization.name}</strong>
                <small>{organization.role} · {organization.mailbox_count} mailbox{organization.mailbox_count === 1 ? '' : 'es'}</small>
              </span>
              {active?.id === organization.id && <Check size={15} />}
            </button>
          ))}

          <form className="business-create" onSubmit={createBusiness}>
            <label htmlFor="business-name">{organizations.length ? 'Create another business' : 'Business name'}</label>
            <div>
              <input
                id="business-name"
                value={name}
                onChange={(event) => setName(event.target.value)}
                placeholder="Company name"
                maxLength={120}
              />
              <button className="icon-button" type="submit" aria-label="Create business" disabled={busy || !name.trim()}>
                <Plus size={16} />
              </button>
            </div>
          </form>
        </aside>

        <div className="business-content">
          {!detail ? (
            <div className="business-empty">
              <Building2 size={28} />
              <h2>Create your business</h2>
              <p>Start with your company identity. Domain ownership and mailbox provisioning are handled separately so your login email never has to match your business domain.</p>
            </div>
          ) : (
            <>
              <section className="business-summary">
                <div>
                  <p className="eyebrow">Active business</p>
                  <h2>{detail.name}</h2>
                  <p>{detail.role === 'owner' ? 'Owner' : detail.role} access · {detail.status}</p>
                </div>
                {detail.is_system && (
                  <span className="business-badge"><ShieldCheck size={14} /> Protected system business</span>
                )}
              </section>

              <section className="business-section" id="business-domains">
                <header>
                  <div><Globe2 size={17} /><h3>Domains</h3></div>
                  <span>{detail.domains.length}</span>
                </header>

                {(detail.role === 'owner' || detail.role === 'admin') && !detail.is_system && (
                  <form className="business-domain-form" onSubmit={claimDomain}>
                    <div>
                      <label htmlFor="business-domain">Add a business domain</label>
                      <input
                        id="business-domain"
                        value={domainName}
                        onChange={(event) => setDomainName(event.target.value)}
                        placeholder="yourcompany.com"
                        autoCapitalize="none"
                        autoCorrect="off"
                        spellCheck={false}
                        required
                      />
                    </div>
                    <button className="primary-button" type="submit" disabled={busy || !domainName.trim()}>
                      <Plus size={15} /> Add domain
                    </button>
                  </form>
                )}

                {domainNotice && <p className="business-domain-notice">{domainNotice}</p>}

                {detail.domains.length ? detail.domains.map((domain) => (
                  <div className="business-domain-card" key={domain.id}>
                    <div className="business-row business-domain-card__head">
                      <div>
                        <strong>{domain.domain}</strong>
                        <small>{domain.is_primary ? 'Primary domain' : 'Domain'} · {domain.status.replaceAll('_', ' ')}</small>
                      </div>
                      <span className={`status-dot ${['verified', 'active'].includes(domain.status) ? 'status-dot--active' : ''}`}>
                        {domain.status.replaceAll('_', ' ')}
                      </span>
                    </div>

                    {domain.status === 'pending_verification' && domain.verification && (
                      <div className="business-dns-proof">
                        <div>
                          <span>DNS record</span>
                          <strong>TXT</strong>
                        </div>
                        <div>
                          <span>Name / host</span>
                          <code>{domain.verification.name}</code>
                          <button type="button" className="icon-button" aria-label="Copy DNS host" onClick={() => void copyText(domain.verification?.name)}><Copy size={14} /></button>
                        </div>
                        <div>
                          <span>Value</span>
                          <code>{domain.verification.value}</code>
                          <button type="button" className="icon-button" aria-label="Copy DNS value" onClick={() => void copyText(domain.verification?.value)}><Copy size={14} /></button>
                        </div>
                        <p>Add this TXT record with your DNS provider. CS Mail only marks the domain verified after the public DNS record matches this challenge.</p>
                        {domain.last_error && <p className="business-domain-error">{domain.last_error}</p>}
                        <div className="business-domain-actions">
                          <button type="button" className="primary-button" disabled={busy} onClick={() => void verifyDomain(domain.id)}>
                            <ShieldCheck size={15} /> Verify DNS
                          </button>
                          <button type="button" className="secondary-button" disabled={busy} onClick={() => void rotateDomain(domain.id)}>
                            <RefreshCw size={15} /> New token
                          </button>
                          <button type="button" className="text-button" disabled={busy} onClick={() => void releaseDomain(domain.id)}>
                            <Trash2 size={14} /> Release claim
                          </button>
                        </div>
                      </div>
                    )}

                    {domain.status === 'verified' && (
                      <div className="business-domain-ready">
                        <ShieldCheck size={16} />
                        <div>
                          <p><strong>Ownership verified.</strong> Provision this domain on CS Mail's mail server to generate its real MX, SPF, DKIM and DMARC records.</p>
                          {(detail.role === 'owner' || detail.role === 'admin') && (
                            <button type="button" className="primary-button" disabled={busy} onClick={() => void provisionDomain(domain.id)}>
                              <Mail size={15} /> Provision mail domain
                            </button>
                          )}
                        </div>
                      </div>
                    )}

                    {['provisioning', 'dns_pending', 'active', 'degraded', 'failed'].includes(domain.status) && domain.provider?.provisioned && domain.dns && (
                      <div className="business-mail-dns">
                        <div className="business-mail-dns__head">
                          <div>
                            <strong>{domain.dns.ready ? 'Mail DNS ready' : 'Mail DNS setup required'}</strong>
                            <p>Publish the provider-generated records below. CS Mail activates business mail only when MX, SPF, DKIM and DMARC all match public DNS.</p>
                          </div>
                          {(detail.role === 'owner' || detail.role === 'admin') && (
                            <button type="button" className="secondary-button" disabled={busy} onClick={() => void checkDomainDns(domain.id)}>
                              <RefreshCw size={15} /> Check DNS
                            </button>
                          )}
                        </div>
                        <div className="business-dns-checks">
                          {[['MX', domain.dns.mx], ['SPF', domain.dns.spf], ['DKIM', domain.dns.dkim], ['DMARC', domain.dns.dmarc]].map(([label, ready]) => (
                            <span key={String(label)} className={ready ? 'business-dns-check business-dns-check--ready' : 'business-dns-check'}>
                              {ready ? <Check size={13} /> : <RefreshCw size={13} />} {String(label)}
                            </span>
                          ))}
                        </div>
                        <div className="business-dns-records">
                          {domain.dns.expected.map((record, index) => (
                            <div className="business-dns-record" key={`${record.kind}-${record.name}-${index}`}>
                              <span>{record.kind}</span>
                              <code>{record.name}</code>
                              <code>{record.priority != null ? `${record.priority} ${record.value}` : record.value}</code>
                              <button type="button" className="icon-button" aria-label="Copy DNS value" onClick={() => void copyText(record.priority != null ? `${record.priority} ${record.value}` : record.value)}><Copy size={14} /></button>
                            </div>
                          ))}
                        </div>
                        <details className="business-zone-file">
                          <summary>Provider DNS zone file</summary>
                          <pre>{domain.dns.zone_file}</pre>
                        </details>
                        {domain.dns.last_checked_at && <small>Last checked {new Date(domain.dns.last_checked_at).toLocaleString()}</small>}
                        {domain.last_error && !domain.dns.ready && <p className="business-domain-error">{domain.last_error}</p>}
                      </div>
                    )}

                    {domain.status === 'failed' && !domain.provider?.provisioned && (detail.role === 'owner' || detail.role === 'admin') && (
                      <div className="business-domain-ready">
                        <p><strong>Provisioning needs attention.</strong> The provider operation is idempotent and safe to retry.</p>
                        <button type="button" className="secondary-button" disabled={busy} onClick={() => void provisionDomain(domain.id)}>Retry provisioning</button>
                      </div>
                    )}
                  </div>
                )) : (
                  <div className="business-placeholder">
                    <strong>No business domain yet</strong>
                    <p>Add the company's domain and publish the generated TXT challenge. CS Mail will not provision mailboxes until DNS ownership has been proven.</p>
                  </div>
                )}
              </section>

              <section className="business-section" id="business-mailboxes">
                <header>
                  <div><Users size={17} /><h3>Mailboxes & storage</h3></div>
                  <span>{detail.mailboxes.length}</span>
                </header>
                {detail.storage && (
                  <div className="business-storage-pool">
                    <div>
                      <span>Business storage pool</span>
                      <strong>{formatStorage(detail.storage.allocated_bytes)} allocated <small>/ {formatStorage(detail.storage.pool_bytes)}</small></strong>
                    </div>
                    <div className="business-storage-pool__meta">
                      <span>{formatStorage(detail.storage.unallocated_bytes)} available</span>
                      {detail.storage.used_bytes != null && <span>{formatStorage(detail.storage.used_bytes)} actually used</span>}
                    </div>
                    <div className="storage-bar"><span style={{ width: `${detail.storage.pool_bytes > 0 ? Math.min(100, (detail.storage.allocated_bytes / detail.storage.pool_bytes) * 100) : 0}%` }} /></div>
                    <p>New mailboxes receive {formatStorage(detail.storage.default_mailbox_bytes)} by default. Owners and admins can redistribute the purchased pool between individual addresses.</p>
                  </div>
                )}
                {(detail.role === 'owner' || detail.role === 'admin') && detail.domains.some((domain) => domain.status === 'active') && (
                  <form className="business-invite-form" onSubmit={createMailbox}>
                    <div><label>Address</label><input value={mailboxLocal} onChange={(e) => setMailboxLocal(e.target.value)} placeholder="name" required /></div>
                    <div><label>Domain</label><select value={mailboxDomainId} onChange={(e) => setMailboxDomainId(e.target.value)}>{detail.domains.filter((d) => d.status === 'active').map((d) => <option key={d.id} value={d.id}>@{d.domain}</option>)}</select></div>
                    <div><label>Assign existing member</label><select value={mailboxMemberId} onChange={(e) => setMailboxMemberId(e.target.value)}><option value="">Invite by email instead</option>{members.filter((m) => m.status === 'active').map((m) => <option key={m.user_id} value={m.user_id}>{m.display_name || m.email}</option>)}</select></div>
                    {!mailboxMemberId && <div><label>Invitation email</label><input type="email" value={mailboxInviteEmail} onChange={(e) => setMailboxInviteEmail(e.target.value)} placeholder="employee@external.com" required /></div>}
                    <button className="primary-button" type="submit" disabled={busy}><Plus size={15}/> Create mailbox</button>
                  </form>
                )}
                {detail.mailboxes.length ? detail.mailboxes.map((mailbox) => (
                  <div className="business-mailbox-row" key={mailbox.id}>
                    <div className="business-row">
                      <div><strong>{mailbox.address}</strong><small>{mailbox.display_name || 'Mailbox'} · {mailbox.sync_status}</small></div>
                      <div className="business-domain-actions">
                        <span className={`status-dot ${mailbox.status === 'active' ? 'status-dot--active' : ''}`}>{mailbox.status}</span>
                        {mailbox.user_id === profile?.id && mailbox.status === 'active' && (
                          activeMailboxId === mailbox.id
                            ? <span className="business-badge"><Check size={13}/> Active mailbox</span>
                            : <button type="button" className="secondary-button" disabled={busy} onClick={() => void switchMailbox(mailbox.id)}>Use mailbox</button>
                        )}
                        {(detail.role === 'owner' || detail.role === 'admin') && mailbox.user_id && mailbox.status !== 'deleting' && <button type="button" className="text-button" disabled={busy} onClick={() => void setMailboxStatus(mailbox.id, mailbox.status === 'suspended' ? 'active' : 'suspended')}>{mailbox.status === 'suspended' ? 'Reactivate' : 'Suspend'}</button>}
                        {(detail.role === 'owner' || detail.role === 'admin') && <button type="button" className="text-button" disabled={busy} onClick={() => void removeMailbox(mailbox.id)}>Delete</button>}
                      </div>
                    </div>
                    {(detail.role === 'owner' || detail.role === 'admin' || mailbox.user_id === profile?.id) && (
                    <div className="business-mailbox-storage">
                      <div className="business-mailbox-storage__readout">
                        <span>Storage</span>
                        <strong>{mailbox.used_bytes != null ? formatStorage(mailbox.used_bytes) : 'Usage unavailable'} <small>/ {formatStorage(mailbox.quota_bytes)}</small></strong>
                        <span className="business-storage-source">{mailbox.quota_source === 'custom' ? 'Custom allocation' : 'Plan default'}</span>
                      </div>
                      <div className="storage-bar"><span style={{ width: `${mailbox.storage_pct ?? 0}%` }} /></div>
                      {(detail.role === 'owner' || detail.role === 'admin') && (
                        <div className="business-storage-editor">
                          <label htmlFor={`quota-${mailbox.id}`}>Allocation</label>
                          <div>
                            <input id={`quota-${mailbox.id}`} type="number" min="0.1" step="0.1" value={mailboxQuotaDrafts[mailbox.id] ?? ''} onChange={(event) => setMailboxQuotaDrafts((current) => ({ ...current, [mailbox.id]: event.target.value }))} />
                            <span>GB</span>
                            <button type="button" className="secondary-button" disabled={busy} onClick={() => void setMailboxStorage(mailbox.id)}>Save</button>
                            {mailbox.quota_source === 'custom' && <button type="button" className="text-button" disabled={busy} onClick={() => void setMailboxStorage(mailbox.id, true)}>Use default</button>}
                          </div>
                        </div>
                      )}
                    </div>
                    )}
                  </div>
                )) : <div className="business-placeholder"><strong>No mailboxes yet</strong><p>Activate a domain, then create addresses for members or invite employees to claim their mailbox.</p></div>}
              </section>

              <section className="business-section">
                <header><div><Mail size={17}/><h3>Aliases & groups</h3></div><span>{addresses.length}</span></header>
                {(detail.role === 'owner' || detail.role === 'admin') && detail.mailboxes.some((m) => m.status === 'active') && (
                  <form className="business-invite-form" onSubmit={createBusinessAddress}>
                    <div><label>Address</label><input value={addressLocal} onChange={(e) => setAddressLocal(e.target.value)} placeholder="sales" required /></div>
                    <div><label>Type</label><select value={addressKind} onChange={(e) => { setAddressKind(e.target.value as 'alias' | 'group'); setAddressMailboxIds([]) }}><option value="alias">Alias</option><option value="group">Group</option></select></div>
                    <div><label>Domain</label><select value={addressDomainId} onChange={(e) => setAddressDomainId(e.target.value)}>{detail.domains.filter((d) => d.status === 'active').map((d) => <option key={d.id} value={d.id}>@{d.domain}</option>)}</select></div>
                    <div><label>Destinations</label><select multiple value={addressMailboxIds} onChange={(e) => { const ids = Array.from(e.currentTarget.selectedOptions).map((o) => o.value); setAddressMailboxIds(addressKind === 'alias' ? ids.slice(-1) : ids) }}>{detail.mailboxes.filter((m) => m.status === 'active').map((m) => <option key={m.id} value={m.id}>{m.address}</option>)}</select></div>
                    <button className="primary-button" type="submit" disabled={busy || addressMailboxIds.length === 0}><Plus size={15}/> Add {addressKind}</button>
                  </form>
                )}
                {addresses.map((address) => <div className="business-row" key={address.id}><div><strong>{address.address}</strong><small>{address.kind} → {address.mailboxes.map((m) => m.address).join(', ')} · {address.sync_status}</small></div>{(detail.role === 'owner' || detail.role === 'admin') && <button type="button" className="text-button" disabled={busy} onClick={() => void removeBusinessAddress(address.id)}>Delete</button>}</div>)}
              </section>

              <section className="business-section">
                <header>
                  <div><Users size={17} /><h3>Team</h3></div>
                  <span>{members.length}</span>
                </header>
                {members.map((member) => (
                  <div className="business-row" key={member.user_id}>
                    <div>
                      <strong>{member.display_name || member.email}</strong>
                      <small>{member.email} · {member.role}</small>
                    </div>
                    <span className={`status-dot ${member.status === 'active' ? 'status-dot--active' : ''}`}>{member.status}</span>
                  </div>
                ))}

                {(detail.role === 'owner' || detail.role === 'admin') && (
                  <div className="business-team-admin">
                    <form className="business-invite-form" onSubmit={inviteMember}>
                      <div>
                        <label htmlFor="invite-email">Invite team member</label>
                        <input
                          id="invite-email"
                          type="email"
                          value={inviteEmail}
                          onChange={(event) => setInviteEmail(event.target.value)}
                          placeholder="person@example.com"
                          required
                        />
                      </div>
                      <div>
                        <label htmlFor="invite-role">Business role</label>
                        <select id="invite-role" value={inviteRole} onChange={(event) => setInviteRole(event.target.value as OrganizationRole)}>
                          <option value="member">Member</option>
                          <option value="billing">Billing</option>
                          <option value="admin">Admin</option>
                          <option value="owner">Owner</option>
                        </select>
                      </div>
                      <button className="primary-button" type="submit" disabled={busy || !inviteEmail.trim()}>
                        <UserPlus size={15} /> Invite
                      </button>
                    </form>

                    {invitations.filter((invitation) => invitation.status === 'pending').length > 0 && (
                      <div className="business-pending">
                        <p className="eyebrow">Pending invitations</p>
                        {invitations.filter((invitation) => invitation.status === 'pending').map((invitation) => (
                          <div className="business-row" key={invitation.id}>
                            <div><strong>{invitation.email}</strong><small>{invitation.role} · expires {new Date(invitation.expires_at).toLocaleDateString()}</small></div>
                            <button type="button" className="text-button" disabled={busy} onClick={() => void revokeInvitation(invitation.id)}>Revoke</button>
                          </div>
                        ))}
                      </div>
                    )}
                  </div>
                )}
              </section>
            </>
          )}
        </div>
      </div>
    </section>
  )
}
