import { apiFetch, tokenStore } from '../lib/api'

export type SupportTicket = {
  id?: string
  reference: string
  requester_name?: string
  requester_email?: string
  topic?: string
  subject: string
  status: 'open' | 'pending' | 'resolved' | 'closed'
  priority?: 'normal' | 'high' | 'urgent'
  created_at?: string
  updated_at?: string
  last_reply_at?: string | null
}

export type SupportMessage = {
  id: string
  author_kind: 'customer' | 'agent' | 'system'
  body: string
  created_at: string
}

export type SupportTicketInput = {
  name: string
  email: string
  topic: string
  subject: string
  message: string
}

export const supportApi = {
  async create(input: SupportTicketInput): Promise<{ reference: string; status: string }> {
    const path = tokenStore.getAccess() ? '/api/support/tickets' : '/api/public/support/tickets'
    return apiFetch<{ reference: string; status: string }>(path, { method: 'POST', body: JSON.stringify(input) }, { retry: Boolean(tokenStore.getAccess()) })
  },

  async mine(): Promise<SupportTicket[]> {
    if (!tokenStore.getAccess()) return []
    const data = await apiFetch<{ tickets: SupportTicket[] }>('/api/support/tickets')
    return data.tickets ?? []
  },
}

export const adminSupportApi = {
  async list(status?: string): Promise<SupportTicket[]> {
    const query = new URLSearchParams()
    if (status && status !== 'all') query.set('status', status)
    const suffix = query.toString() ? `?${query}` : ''
    const data = await apiFetch<{ tickets: SupportTicket[] }>(`/api/admin/support/tickets${suffix}`)
    return data.tickets ?? []
  },

  async get(id: string): Promise<{ ticket: SupportTicket; messages: SupportMessage[] }> {
    return apiFetch<{ ticket: SupportTicket; messages: SupportMessage[] }>(`/api/admin/support/tickets/${encodeURIComponent(id)}`)
  },

  async update(id: string, patch: { status?: string; priority?: string }): Promise<SupportTicket> {
    const data = await apiFetch<{ ticket: SupportTicket }>(`/api/admin/support/tickets/${encodeURIComponent(id)}`, {
      method: 'PATCH',
      body: JSON.stringify(patch),
    })
    return data.ticket
  },

  async reply(id: string, message: string, status = 'pending'): Promise<void> {
    await apiFetch(`/api/admin/support/tickets/${encodeURIComponent(id)}/reply`, {
      method: 'POST',
      body: JSON.stringify({ message, status }),
    })
  },
}
