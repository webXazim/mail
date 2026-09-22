import { apiFetch } from '../lib/api'

export type PublicPlan = {
  code: string
  name: string
  price_cents: number
  extra_mailbox_price_cents: number
  currency: string
  interval: string
  mailbox_bytes: number
  storage_pool_bytes: number
  mailbox_limit: number
  max_mailboxes: number
  alias_limit_per_mailbox: number | null
  domain_limit: number
  organization_daily_send_limit: number
  max_attachment_bytes: number
  max_recipients: number
  daily_send_limit: number
  seats: number
  features: string[]
  feature_flags?: Record<string, boolean>
}

export type PublicStatusComponent = {
  key: string
  name: string
  status: 'operational' | 'degraded' | 'outage'
}

export type PublicIncident = {
  id: string
  title: string
  status: 'investigating' | 'identified' | 'monitoring' | 'resolved'
  impact: 'minor' | 'major' | 'critical'
  message: string
  started_at: string
  resolved_at: string | null
  updated_at: string
}

export type PublicStatus = {
  product: string
  status: 'operational' | 'degraded' | 'major_outage'
  updated_at: string
  components: PublicStatusComponent[]
  active_incidents: number
  incidents: PublicIncident[]
}

export const publicApi = {
  async plans(): Promise<PublicPlan[]> {
    const data = await apiFetch<{ plans: PublicPlan[] }>('/api/public/plans', {}, { retry: false })
    return data.plans ?? []
  },
  status(): Promise<PublicStatus> {
    return apiFetch<PublicStatus>('/api/public/status', {}, { retry: false })
  },
}
