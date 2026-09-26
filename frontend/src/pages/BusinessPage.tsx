import { useEffect, useMemo, useRef, useState, type FormEvent } from 'react'
import { Link } from 'react-router-dom'
import { Building2, Check, Copy, Globe2, Mail, Plus, RefreshCw, ShieldCheck, Trash2, UserPlus, Users } from 'lucide-react'
import { organizationsApi, type BusinessAddress, type OrganizationDetail, type OrganizationInvitation, type OrganizationMember, type OrganizationRole, type OrganizationSummary } from '../services/organizations'
import { profileApi, useProfile } from '../services/profile'
import { beginCloudflareOAuth, clearPendingCloudflareOAuth, readPendingCloudflareOAuth } from '../lib/cloudflare-oauth'
import { ConfirmDialog } from '../components/ConfirmDialog'

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
  const [cloudflareTokens, setCloudflareTokens] = useState<Record<string, string>>({})
  const [cloudflareChecking, setCloudflareChecking] = useState<string | null>(null)
  const cloudflareTimer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const cloudflareRun = useRef(0)
  const cloudflareOAuthHandling = useRef(false)
  const [cloudflareOAuthConfig, setCloudflareOAuthConfig] = useState<Awaited<ReturnType<typeof organizationsApi.cloudflareOAuthConfig>> | null>(null)
  const [mailboxLocal, setMailboxLocal] = useState('')
  const [mailboxDomainId, setMailboxDomainId] = useState('')
  const [mailboxMemberId, setMailboxMemberId] = useState('')
  const [mailboxInviteEmail, setMailboxInviteEmail] = useState('')
  const [mailboxStorageGb, setMailboxStorageGb] = useState('')
  const [mailboxQuotaDrafts, setMailboxQuotaDrafts] = useState<Record<string, string>>({})
  const [addresses, setAddresses] = useState<BusinessAddress[]>([])
  const [addressLocal, setAddressLocal] = useState('')
  const [addressKind, setAddressKind] = useState<'alias' | 'group'>('alias')
  const [addressDomainId, setAddressDomainId] = useState('')
  const [addressMailboxIds, setAddressMailboxIds] = useState<string[]>([])
  const [deleteMailboxTarget, setDeleteMailboxTarget] = useState<{ id: string; address: string; status: string } | null>(null)
  const [deleteDomainTarget, setDeleteDomainTarget] = useState<{ id: string; domain: string } | null>(null)

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
    const timer = window.setTimeout(() => {
      load(readPendingCloudflareOAuth()?.organizationId).catch((cause) => setError(cause instanceof Error ? cause.message : 'Unable to load businesses'))
      organizationsApi.cloudflareOAuthConfig().then(setCloudflareOAuthConfig).catch(() => undefined)
    }, 0)
    return () => window.clearTimeout(timer)
  }, [])

  const startCloudflareConnection = async (domainId: string, mode: 'setup' | 'mail') => {
    if (!detail || !cloudflareOAuthConfig?.available || !cloudflareOAuthConfig.client_id) return
    setError('')
    try {
      await beginCloudflareOAuth({
        client_id: cloudflareOAuthConfig.client_id,
        redirect_uri: cloudflareOAuthConfig.redirect_uri,
        authorization_url: cloudflareOAuthConfig.authorization_url,
      }, detail.id, domainId, mode)
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : 'Unable to connect Cloudflare')
    }
  }

  useEffect(() => () => {
    cloudflareRun.current += 1
    if (cloudflareTimer.current) clearTimeout(cloudflareTimer.current)
  }, [])

  const stopCloudflareCheck = () => {
    cloudflareRun.current += 1
    if (cloudflareTimer.current) clearTimeout(cloudflareTimer.current)
    cloudflareTimer.current = null
    setCloudflareChecking(null)
  }

  useEffect(() => {
    const onRealtime = (event: Event) => {
      const detail = (event as CustomEvent<{ kind?: string; payload?: { resource?: string; organization_id?: string } }>).detail
      if (detail?.kind !== 'resource-changed') return
      const payload = detail.payload
      if (!payload || !['business_domains', 'business_mailboxes'].includes(payload.resource ?? '')) return
      if (activeId && payload.organization_id && payload.organization_id !== activeId) return
      load(activeId || undefined).catch(() => undefined)
    }
    window.addEventListener('cs-mail-realtime', onRealtime)
    return () => window.removeEventListener('cs-mail-realtime', onRealtime)
  }, [activeId])

  const mailboxSetupInProgress = detail?.mailboxes.some((mailbox) =>
    mailbox.status === 'deleting' || (mailbox.user_id && (mailbox.sync_status === 'pending' || mailbox.sync_status === 'retrying'))) ?? false
  useEffect(() => {
    if (!activeId || !mailboxSetupInProgress) return
    let cancelled = false
    const timer = window.setInterval(() => {
      void organizationsApi.mailboxes(activeId).then((result) => {
        if (cancelled) return
        setDetail((current) => current?.id === activeId ? { ...current, mailboxes: result.mailboxes, storage: result.storage } : current)
        setMailboxQuotaDrafts((current) => ({ ...current, ...Object.fromEntries(result.mailboxes.map((mailbox) => [mailbox.id, current[mailbox.id] ?? quotaDraft(mailbox.quota_bytes)])) }))
        if (result.mailboxes.some((mailbox) => mailbox.status === 'active')) void profileApi.refresh()
      }).catch(() => undefined)
    }, 5000)
    return () => { cancelled = true; window.clearInterval(timer) }
  }, [activeId, mailboxSetupInProgress])

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
    stopCloudflareCheck()
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

  const startMailDnsChecks = (organizationId: string, domainId: string, run: number) => {
    let attempts = 0
    const check = async () => {
      if (run !== cloudflareRun.current) return
      attempts += 1
      try {
        const result = await organizationsApi.checkDomainDns(organizationId, domainId)
        if (run !== cloudflareRun.current) return
        await load(organizationId)
        if (result.ready) {
          setCloudflareChecking(null)
          setDomainNotice('Cloudflare mail DNS is verified. The domain is active for business mail.')
          return
        }
      } catch (cause) {
        if (run !== cloudflareRun.current) return
        if (!(cause instanceof Error && /recently|few seconds/i.test(cause.message))) {
          setCloudflareChecking(null)
          setError(cause instanceof Error ? cause.message : 'Unable to check mail DNS')
          return
        }
      }
      if (attempts < 12) {
        cloudflareTimer.current = setTimeout(() => void check(), 20_000)
      } else {
        setCloudflareChecking(null)
        setDomainNotice('Mail records were published in Cloudflare. Public DNS is still propagating; use Check DNS later if the domain does not become active automatically.')
      }
    }
    cloudflareTimer.current = setTimeout(() => void check(), 20_000)
  }

  async function publishCloudflareMailDns(domainId: string, suppliedToken?: string) {
    if (!detail || !(suppliedToken || cloudflareTokens[domainId]?.trim())) return
    const organizationId = detail.id
    const token = suppliedToken || cloudflareTokens[domainId].trim()
    const startingRun = cloudflareRun.current
    setCloudflareTokens((current) => ({ ...current, [domainId]: '' }))
    setBusy(true)
    setError('')
    try {
      const result = await organizationsApi.publishCloudflareMailDns(organizationId, domainId, token)
      if (startingRun !== cloudflareRun.current) return
      stopCloudflareCheck()
      setCloudflareChecking(domainId)
      setDomainNotice(result.message)
      await load(organizationId)
      startMailDnsChecks(organizationId, domainId, cloudflareRun.current)
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : 'Unable to publish Cloudflare mail DNS')
    } finally {
      setBusy(false)
    }
  }

  async function publishCloudflareChallenge(domainId: string, suppliedToken?: string) {
    if (!detail || !(suppliedToken || cloudflareTokens[domainId]?.trim())) return
    const organizationId = detail.id
    const token = suppliedToken || cloudflareTokens[domainId].trim()
    const startingRun = cloudflareRun.current
    setCloudflareTokens((current) => ({ ...current, [domainId]: '' }))
    setBusy(true)
    setError('')
    setDomainNotice('')
    try {
      const published = await organizationsApi.publishCloudflareChallenge(organizationId, domainId, token)
      if (startingRun !== cloudflareRun.current) return
      setDomainNotice(`${published.message} This can take a few minutes.`)
      stopCloudflareCheck()
      setCloudflareChecking(domainId)
      const run = cloudflareRun.current
      let attempts = 0
      const check = async () => {
        if (run !== cloudflareRun.current) return
        attempts += 1
        try {
          const result = await organizationsApi.verifyDomain(organizationId, domainId)
          if (run !== cloudflareRun.current) return
          if (result.verified) {
            setDomainNotice('Ownership verified. Provisioning the mail domain and publishing its Cloudflare DNS records…')
            const provisioned = await organizationsApi.provisionDomain(organizationId, domainId)
            if (run !== cloudflareRun.current) return
            if (!provisioned.domain.provider?.provisioned) {
              throw new Error('Mail domain provisioning did not complete. Use Provision mail domain to retry.')
            }
            await organizationsApi.publishCloudflareMailDns(organizationId, domainId, token)
            if (run !== cloudflareRun.current) return
            setDomainNotice('Ownership verified and mail DNS published in Cloudflare. Checking public DNS…')
            await load(organizationId)
            startMailDnsChecks(organizationId, domainId, run)
            return
          }
        } catch (cause) {
          if (run !== cloudflareRun.current) return
          if (!(cause instanceof Error && /recently|few seconds/i.test(cause.message))) {
            setCloudflareChecking(null)
            setError(cause instanceof Error ? cause.message : 'Unable to check public DNS')
            await load(organizationId).catch(() => undefined)
            return
          }
        }
        if (attempts < 12) {
          cloudflareTimer.current = setTimeout(() => void check(), 20_000)
        } else {
          setCloudflareChecking(null)
          setDomainNotice('Cloudflare record published, but public DNS has not shown it yet. Use Verify DNS later to finish ownership proof.')
        }
      }
      cloudflareTimer.current = setTimeout(() => void check(), 3_000)
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : 'Unable to publish Cloudflare verification record')
    } finally {
      setBusy(false)
    }
  }

  useEffect(() => {
    const timer = window.setTimeout(() => {
      const params = new URLSearchParams(window.location.search)
      if (!params.has('code') && !params.has('error')) return
      const pending = readPendingCloudflareOAuth()
      if (cloudflareOAuthHandling.current) return
      if (!pending) {
        cloudflareOAuthHandling.current = true
        const nextUrl = new URL(window.location.href)
        for (const key of ['code', 'state', 'error', 'error_description']) nextUrl.searchParams.delete(key)
        window.history.replaceState(window.history.state, '', nextUrl.toString())
        setError('Cloudflare setup expired. Connect Cloudflare again to continue.')
        return
      }
      if (!detail) return
      if (detail.id !== pending.organizationId) {
        load(pending.organizationId).catch((cause) => setError(cause instanceof Error ? cause.message : 'Unable to restore business setup'))
        return
      }
      cloudflareOAuthHandling.current = true
      const code = params.get('code')
      const state = params.get('state')
      const nextUrl = new URL(window.location.href)
      for (const key of ['code', 'state', 'error', 'error_description']) nextUrl.searchParams.delete(key)
      window.history.replaceState(window.history.state, '', nextUrl.toString())
      clearPendingCloudflareOAuth()
      if (!code || state !== pending.state || params.has('error')) {
        setError('Cloudflare connection was cancelled or did not match this setup attempt. Please try again.')
        return
      }
      void (async () => {
        try {
          const result = await organizationsApi.exchangeCloudflareCode(pending.organizationId, pending.domainId, code, pending.verifier)
          if (pending.mode === 'setup') await publishCloudflareChallenge(pending.domainId, result.access_token)
          else await publishCloudflareMailDns(pending.domainId, result.access_token)
        } catch (cause) {
          setError(cause instanceof Error ? cause.message : 'Cloudflare connection failed')
        }
      })()
    }, 0)
    return () => window.clearTimeout(timer)
    // The callback intentionally re-runs only when the restored business changes.
    // Publisher helpers use the current render's detail/token state and are declared above.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [detail?.id])

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
    if (cloudflareChecking === domainId) stopCloudflareCheck()
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

  const releaseDomain = async () => {
    if (!detail || !deleteDomainTarget) return
    const domainId = deleteDomainTarget.id
    if (cloudflareChecking === domainId) stopCloudflareCheck()
    setBusy(true)
    setError('')
    try {
      await organizationsApi.releaseDomain(detail.id, domainId)
      setDeleteDomainTarget(null)
      setDomainNotice('Domain deleted from this business. Remove any old DNS records at your DNS provider if you no longer need them.')
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
    const requestedStorageGb = mailboxStorageGb.trim() ? Number(mailboxStorageGb) : null
    if (requestedStorageGb != null && (!Number.isFinite(requestedStorageGb) || requestedStorageGb <= 0)) {
      setError('Enter a valid initial mailbox storage allocation in GB, or leave it blank to use the plan default.')
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
        quota_bytes: requestedStorageGb == null ? undefined : Math.round(requestedStorageGb * GIB),
      })
      setMailboxLocal(''); setMailboxInviteEmail(''); setMailboxStorageGb('')
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

  const retryMailbox = async (mailboxId: string) => {
    if (!detail) return
    setBusy(true); setError('')
    try {
      await organizationsApi.retryMailbox(detail.id, mailboxId)
      await load(detail.id)
    } catch (cause) { setError(cause instanceof Error ? cause.message : 'Unable to retry mailbox setup') }
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

  const distributeAvailableStorage = async () => {
    if (!detail || !detail.storage || detail.storage.unallocated_bytes <= 0) return
    setBusy(true); setError('')
    try {
      await organizationsApi.distributeAvailableStorage(detail.id)
      await load(detail.id)
    } catch (cause) { setError(cause instanceof Error ? cause.message : 'Unable to distribute available storage') }
    finally { setBusy(false) }
  }

  const removeMailbox = async () => {
    if (!detail || !deleteMailboxTarget) return
    const mailboxId = deleteMailboxTarget.id
    setBusy(true); setError('')
    try {
      await organizationsApi.deleteMailbox(detail.id, mailboxId)
      setDeleteMailboxTarget(null)
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
                  <form className="business-form-panel business-domain-form" onSubmit={claimDomain}>
                    <div className="business-field">
                      <label htmlFor="business-domain">Business domain</label>
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
                      <small className="business-field__hint">Enter the root domain you want to use for company mail.</small>
                    </div>
                    <div className="business-form-actions">
                      <button className="primary-button" type="submit" disabled={busy || !domainName.trim()}>
                        <Plus size={15} /> Add domain
                      </button>
                    </div>
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
                      {(detail.role === 'owner' || detail.role === 'admin') && !domain.is_system && (
                        <button type="button" className="secondary-button business-delete-domain" disabled={busy} onClick={() => setDeleteDomainTarget({ id: domain.id, domain: domain.domain })}>
                          <Trash2 size={14} /> Delete domain
                        </button>
                      )}
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
                        {(detail.role === 'owner' || detail.role === 'admin') && <details className="business-cloudflare">
                          <summary>Use Cloudflare to add and verify this record</summary>
                          <p>Authorize Cloudflare to publish the ownership and mail records for this domain. Existing conflicting mail records are never replaced.</p>
                          {cloudflareOAuthConfig?.available
                            ? <button type="button" className="primary-button business-cloudflare-connect" disabled={busy || cloudflareChecking === domain.id} onClick={() => void startCloudflareConnection(domain.id, 'setup')}>Connect Cloudflare and add DNS automatically</button>
                            : <p className="business-cloudflare-unavailable">Cloudflare account connection is not enabled on this installation. A scoped API token can still add DNS automatically below.</p>}
                          <p>{cloudflareOAuthConfig?.available ? 'Or create a ' : 'Create a '}<a href="https://developers.cloudflare.com/fundamentals/api/get-started/create-token/" target="_blank" rel="noreferrer">scoped API token</a> with Zone Read and DNS Write. CS Mail uses it during setup and does not save it.</p>
                          <form onSubmit={(event) => { event.preventDefault(); void publishCloudflareChallenge(domain.id) }}>
                            <label htmlFor={`cloudflare-token-${domain.id}`}>Cloudflare API token</label>
                            <input id={`cloudflare-token-${domain.id}`} type="password" autoComplete="off" spellCheck={false} value={cloudflareTokens[domain.id] ?? ''} onChange={(event) => setCloudflareTokens((current) => ({ ...current, [domain.id]: event.target.value }))} placeholder="Scoped API token" required />
                            <button type="submit" className="secondary-button" disabled={busy || cloudflareChecking === domain.id || !cloudflareTokens[domain.id]?.trim()}>{cloudflareChecking === domain.id ? 'Setting up DNS…' : 'Add DNS automatically'}</button>
                          </form>
                        </details>}
                        {domain.last_error && <p className="business-domain-error">{domain.last_error}</p>}
                        <div className="business-domain-actions">
                          <button type="button" className="primary-button" disabled={busy} onClick={() => void verifyDomain(domain.id)}>
                            <ShieldCheck size={15} /> Verify DNS
                          </button>
                          <button type="button" className="secondary-button" disabled={busy} onClick={() => void rotateDomain(domain.id)}>
                            <RefreshCw size={15} /> New token
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
                        {(detail.role === 'owner' || detail.role === 'admin') && !domain.dns.ready && (
                          <details className="business-cloudflare">
                            <summary>Publish mail DNS with Cloudflare</summary>
                            <p>Authorize Cloudflare to add the provider-generated MX, SPF, DKIM and DMARC records. Existing conflicting mail records must be resolved in Cloudflare first.</p>
                            {cloudflareOAuthConfig?.available
                              ? <button type="button" className="primary-button business-cloudflare-connect" disabled={busy || cloudflareChecking === domain.id} onClick={() => void startCloudflareConnection(domain.id, 'mail')}>Connect Cloudflare and add DNS automatically</button>
                              : <p className="business-cloudflare-unavailable">Cloudflare account connection is not enabled on this installation. A scoped API token can still add DNS automatically below.</p>}
                            <p>{cloudflareOAuthConfig?.available ? 'Or use' : 'Use'} a zone-scoped API token with Zone Read and DNS Write.</p>
                            <form onSubmit={(event) => { event.preventDefault(); void publishCloudflareMailDns(domain.id) }}>
                              <label htmlFor={`cloudflare-mail-token-${domain.id}`}>Cloudflare API token</label>
                              <input id={`cloudflare-mail-token-${domain.id}`} type="password" autoComplete="off" spellCheck={false} value={cloudflareTokens[domain.id] ?? ''} onChange={(event) => setCloudflareTokens((current) => ({ ...current, [domain.id]: event.target.value }))} placeholder="Scoped API token" required />
                              <button type="submit" className="secondary-button" disabled={busy || cloudflareChecking === domain.id || !cloudflareTokens[domain.id]?.trim()}>{cloudflareChecking === domain.id ? 'Checking public DNS…' : 'Publish mail DNS'}</button>
                            </form>
                          </details>
                        )}
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
                        <p>
                          <strong>{domain.last_error?.includes('already exists in the shared mail provider') ? 'Provider domain needs operator review.' : 'Provisioning needs attention.'}</strong>{' '}
                          {domain.last_error || 'The provider operation is safe to retry.'}
                        </p>
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
                  <div><Users size={17} /><h3>{detail.mailboxes.length ? 'Mailboxes & storage' : 'Mailboxes'}</h3></div>
                  <span>{detail.mailboxes.length}</span>
                </header>
                {detail.storage && (
                  <div className="business-storage-pool">
                    <div>
                      <span>Business storage pool</span>
                      <strong>{formatStorage(detail.storage.allocated_bytes)} reserved <small>/ {formatStorage(detail.storage.pool_bytes)}</small></strong>
                    </div>
                    <div className="business-storage-pool__meta">
                      <span>{formatStorage(detail.storage.unallocated_bytes)} unreserved</span>
                      {detail.storage.used_bytes != null && <span>{formatStorage(detail.storage.used_bytes)} used by ready mailboxes</span>}
                    </div>
                    <div className="storage-bar"><span style={{ width: `${detail.storage.pool_bytes > 0 ? Math.min(100, (detail.storage.allocated_bytes / detail.storage.pool_bytes) * 100) : 0}%` }} /></div>
                    <p>Creating a mailbox reserves {formatStorage(detail.storage.default_mailbox_bytes)} by default. You can choose a different initial allocation when creating it, up to the unreserved pool. Storage becomes usable when mail server setup finishes.</p>
                    {(detail.role === 'owner' || detail.role === 'admin') && detail.mailboxes.length > 0 && detail.storage.unallocated_bytes > 0 && <button type="button" className="secondary-button" disabled={busy} onClick={() => void distributeAvailableStorage()}>Distribute available storage</button>}
                  </div>
                )}
                {(detail.role === 'owner' || detail.role === 'admin') && detail.domains.some((domain) => domain.status === 'active') && (
                  <form className="business-form-panel business-mailbox-form" onSubmit={createMailbox}>
                    <div className="business-field business-mailbox-form__address">
                      <label htmlFor="mailbox-local">Mailbox address</label>
                      <div className="business-address-control">
                        <input id="mailbox-local" value={mailboxLocal} onChange={(e) => setMailboxLocal(e.target.value)} placeholder="name" autoCapitalize="none" autoCorrect="off" spellCheck={false} required />
                        <select aria-label="Mailbox domain" value={mailboxDomainId} onChange={(e) => setMailboxDomainId(e.target.value)}>
                          {detail.domains.filter((d) => d.status === 'active').map((d) => <option key={d.id} value={d.id}>@{d.domain}</option>)}
                        </select>
                      </div>
                      <small className="business-field__hint">This is the address the member will send and receive mail from.</small>
                    </div>
                    <div className="business-field">
                      <label htmlFor="mailbox-member">Mailbox owner</label>
                      <select id="mailbox-member" value={mailboxMemberId} onChange={(e) => setMailboxMemberId(e.target.value)}>
                        <option value="">Invite a person by email</option>
                        {members.filter((m) => m.status === 'active').map((m) => <option key={m.user_id} value={m.user_id}>{m.display_name || m.email}</option>)}
                      </select>
                      <small className="business-field__hint">Choose an existing business member, or invite someone new.</small>
                    </div>
                    {!mailboxMemberId && (
                      <div className="business-field">
                        <label htmlFor="mailbox-invite-email">Invitation email</label>
                        <input id="mailbox-invite-email" type="email" value={mailboxInviteEmail} onChange={(e) => setMailboxInviteEmail(e.target.value)} placeholder="employee@external.com" autoComplete="email" required />
                        <small className="business-field__hint">The invitation is sent here; it does not need to match the mailbox address.</small>
                      </div>
                    )}
                    <div className="business-field">
                      <label htmlFor="mailbox-storage">Initial storage</label>
                      <div className="business-input-suffix">
                        <input id="mailbox-storage" type="number" min="0.1" step="0.1" value={mailboxStorageGb} onChange={(e) => setMailboxStorageGb(e.target.value)} placeholder={detail.storage ? quotaDraft(detail.storage.default_mailbox_bytes) : 'Plan default'} />
                        <span>GB</span>
                      </div>
                      <small className="business-field__hint">{detail.storage ? `${formatStorage(detail.storage.unallocated_bytes)} unreserved · blank uses the ${formatStorage(detail.storage.default_mailbox_bytes)} plan default.` : 'Leave blank to use the plan default.'}</small>
                    </div>
                    <div className="business-form-actions business-mailbox-form__actions">
                      <button className="primary-button" type="submit" disabled={busy || !mailboxLocal.trim() || !mailboxDomainId || (!mailboxMemberId && !mailboxInviteEmail.trim())}><Plus size={15}/> Create mailbox</button>
                    </div>
                  </form>
                )}
                <p className="business-mailbox-help">A mailbox uses your CS Mail sign-in for webmail. You do not enter your account password when creating an address. For IMAP or SMTP, the mailbox owner creates a separate app password in <Link to="/mail/settings?tab=clients">Mail clients</Link> after setup completes.</p>
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
                        {(detail.role === 'owner' || detail.role === 'admin') && mailbox.user_id && (mailbox.status === 'active' || mailbox.status === 'suspended') && <button type="button" className="text-button" disabled={busy} onClick={() => void setMailboxStatus(mailbox.id, mailbox.status === 'suspended' ? 'active' : 'suspended')}>{mailbox.status === 'suspended' ? 'Reactivate' : 'Suspend'}</button>}
                        {(detail.role === 'owner' || detail.role === 'admin') && mailbox.user_id && mailbox.status === 'error' && <button type="button" className="secondary-button" disabled={busy} onClick={() => void retryMailbox(mailbox.id)}>Retry setup</button>}
                        {(detail.role === 'owner' || detail.role === 'admin') && <button type="button" className="text-button" disabled={busy} onClick={() => setDeleteMailboxTarget({ id: mailbox.id, address: mailbox.address, status: mailbox.status })}>{mailbox.status === 'deleting' ? 'Retry delete' : 'Delete'}</button>}
                      </div>
                    </div>
                    {mailbox.sync_error && <p className="business-mailbox-notice" role="status">{mailbox.sync_error}</p>}
                    {mailbox.status !== 'active' && !mailbox.sync_error && <p className="business-mailbox-notice" role="status">{mailbox.user_id ? 'Mail server setup is in progress. Storage is reserved until the mailbox is ready.' : 'Waiting for the invited member to accept before mail server setup begins.'}</p>}
                    {(detail.role === 'owner' || detail.role === 'admin' || mailbox.user_id === profile?.id) && (
                    <div className="business-mailbox-storage">
                      <div className="business-mailbox-storage__readout">
                        <span>Storage</span>
                        <strong>{mailbox.status === 'active' ? (mailbox.used_bytes != null ? formatStorage(mailbox.used_bytes) : 'Usage unavailable') : 'Not active'} <small>/ {formatStorage(mailbox.quota_bytes)} reserved</small></strong>
                        <span className="business-storage-source">{mailbox.quota_source === 'custom' ? 'Custom allocation' : 'Plan default'}{mailbox.status === 'active' && mailbox.quota_in_sync === false ? ' · Provider quota syncing' : ''}</span>
                      </div>
                      <div className="storage-bar"><span style={{ width: `${mailbox.storage_pct ?? 0}%` }} /></div>
                      {(detail.role === 'owner' || detail.role === 'admin') && (
                        <div className="business-storage-editor">
                          <label htmlFor={`quota-${mailbox.id}`}>Allocation</label>
                          <div>
                            <div className="business-input-suffix business-input-suffix--compact">
                              <input id={`quota-${mailbox.id}`} type="number" min="0.1" step="0.1" value={mailboxQuotaDrafts[mailbox.id] ?? ''} onChange={(event) => setMailboxQuotaDrafts((current) => ({ ...current, [mailbox.id]: event.target.value }))} />
                              <span>GB</span>
                            </div>
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
                  <form className="business-form-panel business-address-form" onSubmit={createBusinessAddress}>
                    <div className="business-field business-address-form__address">
                      <label htmlFor="address-local">Address</label>
                      <div className="business-address-control">
                        <input id="address-local" value={addressLocal} onChange={(e) => setAddressLocal(e.target.value)} placeholder="sales" autoCapitalize="none" autoCorrect="off" spellCheck={false} required />
                        <select aria-label="Address domain" value={addressDomainId} onChange={(e) => setAddressDomainId(e.target.value)}>
                          {detail.domains.filter((d) => d.status === 'active').map((d) => <option key={d.id} value={d.id}>@{d.domain}</option>)}
                        </select>
                      </div>
                      <small className="business-field__hint">Create a shared address without another mailbox.</small>
                    </div>
                    <div className="business-field">
                      <label htmlFor="address-kind">Type</label>
                      <select id="address-kind" value={addressKind} onChange={(e) => { setAddressKind(e.target.value as 'alias' | 'group'); setAddressMailboxIds([]) }}>
                        <option value="alias">Alias</option>
                        <option value="group">Group</option>
                      </select>
                      <small className="business-field__hint">{addressKind === 'alias' ? 'Delivers to one mailbox.' : 'Delivers to multiple mailboxes.'}</small>
                    </div>
                    <div className="business-field business-address-form__destinations">
                      <label htmlFor="address-destinations">Destinations</label>
                      <select id="address-destinations" className="business-multi-select" multiple value={addressMailboxIds} onChange={(e) => { const ids = Array.from(e.currentTarget.selectedOptions).map((o) => o.value); setAddressMailboxIds(addressKind === 'alias' ? ids.slice(-1) : ids) }}>
                        {detail.mailboxes.filter((m) => m.status === 'active').map((m) => <option key={m.id} value={m.id}>{m.address}</option>)}
                      </select>
                      <small className="business-field__hint">{addressKind === 'alias' ? 'Select the mailbox that receives this alias.' : 'Use Ctrl/Cmd to select more than one mailbox.'}</small>
                    </div>
                    <div className="business-form-actions business-address-form__actions">
                      <button className="primary-button" type="submit" disabled={busy || !addressLocal.trim() || addressMailboxIds.length === 0}><Plus size={15}/> Add {addressKind}</button>
                    </div>
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
                    <form className="business-form-panel business-team-form" onSubmit={inviteMember}>
                      <div className="business-field">
                        <label htmlFor="invite-email">Team member email</label>
                        <input
                          id="invite-email"
                          type="email"
                          value={inviteEmail}
                          onChange={(event) => setInviteEmail(event.target.value)}
                          placeholder="person@example.com"
                          autoComplete="email"
                          required
                        />
                        <small className="business-field__hint">They will receive an invitation to join this business.</small>
                      </div>
                      <div className="business-field">
                        <label htmlFor="invite-role">Business role</label>
                        <select id="invite-role" value={inviteRole} onChange={(event) => setInviteRole(event.target.value as OrganizationRole)}>
                          <option value="member">Member</option>
                          <option value="billing">Billing</option>
                          <option value="admin">Admin</option>
                          <option value="owner">Owner</option>
                        </select>
                        <small className="business-field__hint">Controls business-level permissions after acceptance.</small>
                      </div>
                      <div className="business-form-actions">
                        <button className="primary-button" type="submit" disabled={busy || !inviteEmail.trim()}>
                          <UserPlus size={15} /> Invite
                        </button>
                      </div>
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
      {deleteMailboxTarget && <ConfirmDialog key={deleteMailboxTarget.id} open danger busy={busy}
        title={deleteMailboxTarget.status === 'deleting' ? 'Retry permanent mailbox deletion' : 'Permanently delete mailbox'}
        description={`Delete ${deleteMailboxTarget.address} and all mailbox-owned data? This cannot be undone.`}
        details={['The hosted Stalwart account and all messages are permanently removed.','Mailbox-scoped drafts, schedules, contacts, rules, app passwords and database rows are removed.','Attachment objects and mailbox import files are removed from configured storage.','Aliases/groups are detached automatically; empty addresses are removed.','Security/audit history is retained and may contain historical identifiers, but not mailbox message content.']}
        verificationText={deleteMailboxTarget.address} confirmLabel={deleteMailboxTarget.status === 'deleting' ? 'Retry deletion' : 'Delete mailbox'}
        onClose={() => setDeleteMailboxTarget(null)} onConfirm={removeMailbox}/>}
      {deleteDomainTarget && <ConfirmDialog key={deleteDomainTarget.id} open danger busy={busy} title="Delete business domain"
        description={`Delete ${deleteDomainTarget.domain} from this business? Hosted mailboxes and aliases must already be removed.`}
        details={['DNS records at your DNS provider are not deleted automatically.']} verificationText={deleteDomainTarget.domain} confirmLabel="Delete domain"
        onClose={() => setDeleteDomainTarget(null)} onConfirm={releaseDomain}/>}
    </section>
  )
}
