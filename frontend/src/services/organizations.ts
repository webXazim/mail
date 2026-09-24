import { apiFetch, mailboxContextStore } from '../lib/api'

export type OrganizationRole = 'owner' | 'admin' | 'billing' | 'member'
export type OrganizationSummary = {
  id: string
  name: string
  slug: string
  status: 'active' | 'suspended' | 'closed'
  is_system: boolean
  role: OrganizationRole
  member_count: number
  domain_count: number
  active_domain_count: number
  mailbox_count: number
}

export type OrganizationDomain = {
  id: string
  organization_id?: string
  domain: string
  status: string
  is_primary: boolean
  is_system: boolean
  verified_at?: string | null
  verification?: {
    type: 'TXT'
    name: string | null
    value: string | null
    expires_at: string | null
    attempts: number
    last_checked_at: string | null
    last_observed_value: string
  }
  provider?: {
    provisioned: boolean
    synced_at: string | null
  }
  dns?: {
    zone_file: string
    expected: Array<{ kind: string; name: string; value: string; priority?: number }>
    observed: Record<string, unknown>
    mx: boolean
    spf: boolean
    dkim: boolean
    dmarc: boolean
    ready: boolean
    last_checked_at: string | null
  }
  last_error: string
}

export type OrganizationMailbox = {
  id: string
  domain_id?: string
  address: string
  display_name: string
  user_id: string | null
  invited_email?: string | null
  status: string
  sync_status: string
  sync_error?: string
  quota_bytes?: number | null
  quota_override_bytes?: number | null
  quota_source?: 'default' | 'custom'
  used_bytes?: number | null
  provider_quota_bytes?: number | null
  quota_in_sync?: boolean | null
  storage_pct?: number | null
}

export type OrganizationStorage = {
  pool_bytes: number
  allocated_bytes: number
  unallocated_bytes: number
  used_bytes: number | null
  default_mailbox_bytes: number
  can_manage: boolean
}

export type OrganizationDetail = {
  id: string
  name: string
  slug: string
  status: string
  is_system: boolean
  role: OrganizationRole
  domains: OrganizationDomain[]
  mailboxes: OrganizationMailbox[]
  storage?: OrganizationStorage | null
}

export type OrganizationMember = {
  user_id: string
  email: string
  display_name: string
  role: OrganizationRole
  status: string
  joined_at: string
}


export type MailboxInvitation = {
  id: string
  mailbox_id: string
  email: string
  role: OrganizationRole
  status: string
  expires_at: string
  created_at: string
}

export type BusinessAddress = {
  id: string
  domain_id: string
  address: string
  local_part: string
  kind: 'alias' | 'group'
  enabled: boolean
  sync_status: string
  sync_error: string
  mailboxes: Array<{ id: string; address: string }>
}
export type OrganizationInvitation = {
  id: string
  email: string
  role: OrganizationRole
  status: string
  expires_at: string
  created_at: string
}

export const organizationsApi = {
  list: () =>
    apiFetch<{
      organizations: OrganizationSummary[]
      active_organization_id: string | null
      active_mailbox_id: string | null
      primary_mailbox_id: string | null
    }>('/api/organizations'),
  create: (name: string) =>
    apiFetch<{ id: string; name: string; slug: string; role: OrganizationRole; status: string }>(
      '/api/organizations',
      { method: 'POST', body: JSON.stringify({ name }) },
    ),
  get: (id: string) => apiFetch<OrganizationDetail>(`/api/organizations/${id}`),
  domains: (id: string) =>
    apiFetch<{ domains: OrganizationDomain[] }>(`/api/organizations/${id}/domains`),
  claimDomain: (id: string, domain: string) =>
    apiFetch<OrganizationDomain>(`/api/organizations/${id}/domains`, {
      method: 'POST',
      body: JSON.stringify({ domain }),
    }),
  verifyDomain: (organizationId: string, domainId: string) =>
    apiFetch<{ verified: boolean; domain: OrganizationDomain; message: string }>(
      `/api/organizations/${organizationId}/domains/${domainId}/verify`,
      { method: 'POST' },
    ),
  publishCloudflareChallenge: (organizationId: string, domainId: string, apiToken: string) =>
    apiFetch<{ ok: true; zone: string; message: string }>(
      `/api/organizations/${organizationId}/domains/${domainId}/cloudflare-txt`,
      { method: 'POST', body: JSON.stringify({ api_token: apiToken }) },
    ),
  publishCloudflareMailDns: (organizationId: string, domainId: string, apiToken: string) =>
    apiFetch<{ ok: true; zone: string; created: number; message: string }>(
      `/api/organizations/${organizationId}/domains/${domainId}/cloudflare-mail-dns`,
      { method: 'POST', body: JSON.stringify({ api_token: apiToken }) },
    ),
  rotateDomainChallenge: (organizationId: string, domainId: string) =>
    apiFetch<OrganizationDomain>(
      `/api/organizations/${organizationId}/domains/${domainId}/challenge`,
      { method: 'POST' },
    ),
  provisionDomain: (organizationId: string, domainId: string) =>
    apiFetch<{ domain: OrganizationDomain; message: string }>(
      `/api/organizations/${organizationId}/domains/${domainId}/provision`,
      { method: 'POST' },
    ),
  checkDomainDns: (organizationId: string, domainId: string) =>
    apiFetch<{ ready: boolean; domain: OrganizationDomain; message: string }>(
      `/api/organizations/${organizationId}/domains/${domainId}/dns-check`,
      { method: 'POST' },
    ),
  releaseDomain: (organizationId: string, domainId: string) =>
    apiFetch<{ ok: true }>(`/api/organizations/${organizationId}/domains/${domainId}`, {
      method: 'DELETE',
    }),
  update: (id: string, name: string) =>
    apiFetch<OrganizationDetail>(`/api/organizations/${id}`, {
      method: 'PATCH',
      body: JSON.stringify({ name }),
    }),
  activate: async (id: string) => {
    const result = await apiFetch<{ ok: true; active_organization_id: string; active_mailbox_id: string | null }>(`/api/organizations/${id}/activate`, {
      method: 'POST',
    })
    mailboxContextStore.set(result.active_organization_id, result.active_mailbox_id)
    return result
  },
  members: (id: string) =>
    apiFetch<{ members: OrganizationMember[] }>(`/api/organizations/${id}/members`),
  invitations: (id: string) =>
    apiFetch<{ invitations: OrganizationInvitation[] }>(`/api/organizations/${id}/invitations`),
  invite: (id: string, email: string, role: OrganizationRole) =>
    apiFetch<{ ok: true; id: string; expires_in_days: number }>(`/api/organizations/${id}/invitations`, {
      method: 'POST',
      body: JSON.stringify({ email, role }),
    }),
  revokeInvitation: (organizationId: string, invitationId: string) =>
    apiFetch<{ ok: true }>(
      `/api/organizations/${organizationId}/invitations/${invitationId}`,
      { method: 'DELETE' },
    ),
  acceptInvitation: (token: string) =>
    apiFetch<{ ok: true; organization_id: string; role: OrganizationRole }>(
      '/api/organization-invitations/accept',
      { method: 'POST', body: JSON.stringify({ token }) },
    ),
  mailboxes: (id: string) => apiFetch<{ mailboxes: OrganizationMailbox[]; storage: OrganizationStorage | null }>(`/api/organizations/${id}/mailboxes`),
  createMailbox: (id: string, payload: { domain_id: string; local_part: string; display_name?: string; member_user_id?: string; invite_email?: string; role?: OrganizationRole }) =>
    apiFetch<{ id: string; address: string; status: string; user_id: string | null }>(`/api/organizations/${id}/mailboxes`, { method: 'POST', body: JSON.stringify(payload) }),
  activateMailbox: async (organizationId: string, mailboxId: string) => {
    const result = await apiFetch<{ ok: true; active_organization_id: string; active_mailbox_id: string; address: string }>(`/api/organizations/${organizationId}/mailboxes/${mailboxId}/activate`, { method: 'POST' })
    mailboxContextStore.set(result.active_organization_id, result.active_mailbox_id)
    return result
  },
  updateMailbox: (organizationId: string, mailboxId: string, status: 'active' | 'suspended', display_name = '') =>
    apiFetch<{ ok: true; status: string }>(`/api/organizations/${organizationId}/mailboxes/${mailboxId}`, { method: 'PATCH', body: JSON.stringify({ status, display_name }) }),
  updateMailboxStorage: (organizationId: string, mailboxId: string, quotaBytes?: number, resetToDefault = false) =>
    apiFetch<{ ok: true; mailbox_id: string; quota_bytes: number; quota_source: 'default' | 'custom' }>(`/api/organizations/${organizationId}/mailboxes/${mailboxId}/storage`, {
      method: 'PATCH',
      body: JSON.stringify(resetToDefault ? { reset_to_default: true } : { quota_bytes: quotaBytes }),
    }),
  deleteMailbox: (organizationId: string, mailboxId: string) =>
    apiFetch<{ ok: true; status: string }>(`/api/organizations/${organizationId}/mailboxes/${mailboxId}`, { method: 'DELETE' }),
  mailboxInvitations: (id: string) => apiFetch<{ invitations: MailboxInvitation[] }>(`/api/organizations/${id}/mailbox-invitations`),
  revokeMailboxInvitation: (organizationId: string, invitationId: string) =>
    apiFetch<{ ok: true }>(`/api/organizations/${organizationId}/mailbox-invitations/${invitationId}`, { method: 'DELETE' }),
  acceptMailboxInvitation: (token: string) =>
    apiFetch<{ ok: true; organization_id: string; mailbox_id: string }>('/api/mailbox-invitations/accept', { method: 'POST', body: JSON.stringify({ token }) }),
  addresses: (id: string) => apiFetch<{ addresses: BusinessAddress[] }>(`/api/organizations/${id}/addresses`),
  createAddress: (id: string, payload: { domain_id: string; local_part: string; kind: 'alias' | 'group'; mailbox_ids: string[] }) =>
    apiFetch<{ id: string; address: string; kind: string; sync: unknown }>(`/api/organizations/${id}/addresses`, { method: 'POST', body: JSON.stringify(payload) }),
  updateAddress: (organizationId: string, addressId: string, payload: { enabled?: boolean; mailbox_ids?: string[] }) =>
    apiFetch<{ ok: true; sync: unknown }>(`/api/organizations/${organizationId}/addresses/${addressId}`, { method: 'PATCH', body: JSON.stringify(payload) }),
  deleteAddress: (organizationId: string, addressId: string) =>
    apiFetch<{ ok: true; sync: unknown }>(`/api/organizations/${organizationId}/addresses/${addressId}`, { method: 'DELETE' }),
}
