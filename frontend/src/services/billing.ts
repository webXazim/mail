import { apiFetch, ApiError } from '../lib/api'
import { registerInvoiceResolver, type Invoice } from '../lib/invoices'
import { isRemoteMail } from './remote-mail'

export type PlanView = {
  code: string
  name: string
  price_cents: number
  price: string
  currency: string
  interval: string
  mailbox_bytes: number
  max_attachment_bytes: number
  max_recipients: number
  daily_send_limit: number
  seats: number
  features: string[]
  active: boolean
}

export type OrderRow = {
  id: string
  user_id: string
  email: string
  display_name: string
  plan_code: string
  plan_name: string
  amount_cents: number
  currency: string
  interval: string
  seats: number
  status: 'pending' | 'submitted' | 'paid' | 'cancelled' | 'rejected'
  payment_method: string
  payment_reference: string
  customer_note: string
  admin_note: string
  invoice_number: string | null
  created_at: string
  submitted_at: string | null
  paid_at: string | null
}

export type BillingSettingsRow = {
  bank_details: string
  paypal_email: string
  instructions: string
}

export type BillingSummary = {
  current_plan: PlanView
  quota_bytes: number
  settings: BillingSettingsRow
  orders: OrderRow[]
}

export const paymentMethodLabel = (method: string) =>
  ({ bank: 'Bank transfer', paypal: 'PayPal', card: 'Card', other: 'Other' })[method] ?? method

export function formatPrice(cents: number, currency: string): string {
  const symbol =
    { USD: '$', EUR: '€', GBP: '£' }[currency.toUpperCase()] ?? `${currency.toUpperCase()} `
  return `${symbol}${Math.floor(cents / 100)}.${String(cents % 100).padStart(2, '0')}`
}

export function splitBytes(bytes: number): { value: string; unit: string } {
  if (bytes >= 1024 * 1024 * 1024)
    return {
      value: (bytes / (1024 * 1024 * 1024)).toFixed(bytes % (1024 * 1024 * 1024) ? 1 : 0),
      unit: 'GB',
    }
  if (bytes >= 1024 * 1024) return { value: `${Math.round(bytes / (1024 * 1024))}`, unit: 'MB' }
  return { value: `${Math.round(bytes / 1024)}`, unit: 'KB' }
}

export function summaryText(limit: number): string {
  if (limit === 0) return 'Unlimited'
  return `${limit.toLocaleString()} / day`
}

const DEMO_PLANS: PlanView[] = [
  {
    code: 'solo',
    name: 'Harbor Solo',
    price_cents: 0,
    price: '$0.00',
    currency: 'USD',
    interval: 'month',
    mailbox_bytes: 2 * 1024 * 1024 * 1024,
    max_attachment_bytes: 25 * 1024 * 1024,
    max_recipients: 50,
    daily_send_limit: 0,
    seats: 1,
    features: [
      'Personal email',
      '2 GB of mailbox storage',
      '25 MB attachments',
      'Up to 50 recipients',
    ],
    active: true,
  },
  {
    code: 'team',
    name: 'Harbor Team',
    price_cents: 800,
    price: '$8.00',
    currency: 'USD',
    interval: 'month',
    mailbox_bytes: 25 * 1024 * 1024 * 1024,
    max_attachment_bytes: 25 * 1024 * 1024,
    max_recipients: 100,
    daily_send_limit: 500,
    seats: 5,
    features: [
      'Everything in Solo',
      '25 GB of mailbox storage',
      'Up to 100 recipients',
      '500 sends / day',
    ],
    active: true,
  },
  {
    code: 'business',
    name: 'Harbor Business',
    price_cents: 1600,
    price: '$16.00',
    currency: 'USD',
    interval: 'month',
    mailbox_bytes: 100 * 1024 * 1024 * 1024,
    max_attachment_bytes: 50 * 1024 * 1024,
    max_recipients: 500,
    daily_send_limit: 5000,
    seats: 25,
    features: [
      'Everything in Team',
      '100 GB of mailbox storage',
      '50 MB attachments',
      'Up to 500 recipients',
      '5,000 sends / day',
    ],
    active: true,
  },
]

const DEMO_SETTINGS: BillingSettingsRow = {
  bank_details:
    'Harbor Mail Ltd · Deutsche Bank · IBAN DE89 3704 0044 0532 0130 00 · BIC COBADEFFXXX',
  paypal_email: 'billing@harbor.co',
  instructions:
    'After you place the order, send the payment and come back to mark it paid with your bank/PayPal reference. An admin verifies the payment and activates your plan — you will see a confirmation and invoice right here.',
}

const nowIso = () => new Date().toISOString()
const newId = () => (isRemoteMail() ? '' : `demo-${Date.now()}`)

let demoOrders: OrderRow[] = []
let demoPlans: PlanView[] = [...DEMO_PLANS]
let demoSettings: BillingSettingsRow = { ...DEMO_SETTINGS }

function demoOrder(plan: PlanView, method: string, note: string): OrderRow {
  return {
    id: newId(),
    user_id: 'demo',
    email: 'you@harbor.co',
    display_name: 'You',
    plan_code: plan.code,
    plan_name: plan.name,
    amount_cents: plan.price_cents,
    currency: plan.currency,
    interval: plan.interval,
    seats: 1,
    status: 'pending',
    payment_method: method,
    payment_reference: '',
    customer_note: note,
    admin_note: '',
    invoice_number: null,
    created_at: nowIso(),
    submitted_at: null,
    paid_at: null,
  }
}

export function invoiceFromOrder(order: OrderRow): Invoice | null {
  if (!order.invoice_number) return null
  const amount = formatPrice(order.amount_cents, order.currency)
  const paidAt = order.paid_at ? new Date(order.paid_at) : new Date()
  const period = paidAt.toLocaleDateString([], { month: 'long', year: 'numeric' })
  return {
    id: order.invoice_number,
    period,
    date: paidAt.toLocaleDateString([], { month: 'long', day: 'numeric', year: 'numeric' }),
    amount,
    status: 'paid',
    plan: order.plan_name,
    seats: order.seats,
    rate: amount,
    subtotal: amount,
    tax: '$0.00',
    total: amount,
    paymentMethod: paymentMethodLabel(order.payment_method),
    billedTo: order.email,
  }
}

export const billingApi = {
  async summary(): Promise<BillingSummary> {
    if (!isRemoteMail()) {
      return {
        current_plan: DEMO_PLANS[1] ?? demoPlans[0],
        quota_bytes: 25 * 1024 * 1024 * 1024,
        settings: demoSettings,
        orders: demoOrders,
      }
    }
    return apiFetch<BillingSummary>('/api/billing')
  },
  async plans(): Promise<PlanView[]> {
    if (!isRemoteMail()) return demoPlans
    const data = await apiFetch<{ plans: PlanView[] }>('/api/billing/plans')
    return data.plans
  },
  async createOrder(
    planCode: string,
    paymentMethod: string,
    customerNote: string,
  ): Promise<OrderRow> {
    if (!isRemoteMail()) {
      const plan = demoPlans.find((item) => item.code === planCode) ?? demoPlans[0]
      const order = demoOrder(plan, paymentMethod, customerNote)
      demoOrders = [order, ...demoOrders]
      return order
    }
    return apiFetch<OrderRow>('/api/billing/orders', {
      method: 'POST',
      body: JSON.stringify({
        plan_code: planCode,
        payment_method: paymentMethod,
        customer_note: customerNote,
      }),
    })
  },
  async submitPaid(orderId: string, paymentMethod: string, reference: string): Promise<OrderRow> {
    if (!isRemoteMail()) {
      demoOrders = demoOrders.map((order) =>
        order.id === orderId
          ? { ...order, status: 'submitted', payment_reference: reference, submitted_at: nowIso() }
          : order,
      )
      return demoOrders.find((order) => order.id === orderId)!
    }
    return apiFetch<OrderRow>(`/api/billing/orders/${encodeURIComponent(orderId)}/paid`, {
      method: 'POST',
      body: JSON.stringify({ payment_method: paymentMethod, payment_reference: reference }),
    })
  },
  async cancelOrder(orderId: string): Promise<void> {
    if (!isRemoteMail()) {
      demoOrders = demoOrders.map((order) =>
        order.id === orderId ? { ...order, status: 'cancelled', payment_reference: '' } : order,
      )
      return
    }
    await apiFetch(`/api/billing/orders/${encodeURIComponent(orderId)}/cancel`, { method: 'POST' })
  },
  async invoices(): Promise<OrderRow[]> {
    const summary = await billingApi.summary()
    return summary.orders.filter((order) => order.status === 'paid')
  },
  invoiceFor(id: string): Invoice | null {
    const order = demoOrders.find(
      (item) => item.status === 'paid' && item.invoice_number?.toLowerCase() === id.toLowerCase(),
    )
    return order ? invoiceFromOrder(order) : null
  },
  refreshInvoices() {
    void billingApi.invoices().then((paid) => registerInvoiceResolverCached(paid))
  },
}

let cachedInvoices: Invoice[] = []
function registerInvoiceResolverCached(paid: OrderRow[]) {
  cachedInvoices = paid
    .map((order) => invoiceFromOrder(order))
    .filter((invoice): invoice is Invoice => invoice !== null)
  registerInvoiceResolver((id) =>
    cachedInvoices.find((invoice) => invoice.id.toLowerCase() === id.toLowerCase()),
  )
}

export const adminBillingApi = {
  async plans(): Promise<PlanView[]> {
    if (!isRemoteMail()) return demoPlans
    const data = await apiFetch<{ plans: PlanView[] }>('/api/admin/plans')
    return data.plans
  },
  async createPlan(input: PlanView): Promise<void> {
    if (!isRemoteMail()) {
      demoPlans = [...demoPlans, { ...input, code: input.code.toLowerCase().replace(/\s+/g, '-') }]
      return
    }
    await apiFetch('/api/admin/plans', { method: 'POST', body: JSON.stringify(input) })
  },
  async updatePlan(code: string, input: PlanView): Promise<void> {
    if (!isRemoteMail()) {
      demoPlans = demoPlans.map((plan) => (plan.code === code ? { ...input, code } : plan))
      return
    }
    await apiFetch(`/api/admin/plans/${encodeURIComponent(code)}`, {
      method: 'PATCH',
      body: JSON.stringify(input),
    })
  },
  async deactivatePlan(code: string): Promise<void> {
    if (!isRemoteMail()) {
      demoPlans = demoPlans.map((plan) => (plan.code === code ? { ...plan, active: false } : plan))
      return
    }
    await apiFetch(`/api/admin/plans/${encodeURIComponent(code)}`, { method: 'DELETE' })
  },
  async orders(status?: string): Promise<OrderRow[]> {
    if (!isRemoteMail()) return demoOrders
    const query = status && status !== 'queue' ? `?status=${encodeURIComponent(status)}` : ''
    const data = await apiFetch<{ orders: OrderRow[] }>(`/api/admin/orders${query}`)
    return data.orders
  },
  async approveOrder(orderId: string, adminNote: string): Promise<void> {
    if (!isRemoteMail()) {
      const order = demoOrders.find((item) => item.id === orderId)
      if (order) {
        const paid = {
          ...order,
          status: 'paid' as const,
          admin_note: adminNote,
          invoice_number: `INV-${Date.now()}`,
          paid_at: nowIso(),
        }
        const invoice = invoiceFromOrder(paid)
        const without = cachedInvoices.filter((inv) => inv.id !== order.invoice_number)
        cachedInvoices = invoice ? [invoice, ...without] : without
        demoOrders = demoOrders.map((item) => (item.id === orderId ? paid : item))
      }
      return
    }
    await apiFetch(`/api/admin/orders/${encodeURIComponent(orderId)}/approve`, {
      method: 'POST',
      body: JSON.stringify({ admin_note: adminNote }),
    })
  },
  async rejectOrder(orderId: string, adminNote: string): Promise<void> {
    if (!isRemoteMail()) {
      demoOrders = demoOrders.map((order) =>
        order.id === orderId ? { ...order, status: 'rejected', admin_note: adminNote } : order,
      )
      return
    }
    await apiFetch(`/api/admin/orders/${encodeURIComponent(orderId)}/reject`, {
      method: 'POST',
      body: JSON.stringify({ admin_note: adminNote }),
    })
  },
  async settings(): Promise<BillingSettingsRow> {
    if (!isRemoteMail()) return demoSettings
    return apiFetch<BillingSettingsRow>('/api/admin/billing-settings')
  },
  async updateSettings(next: BillingSettingsRow): Promise<void> {
    if (!isRemoteMail()) {
      demoSettings = { ...next }
      return
    }
    await apiFetch('/api/admin/billing-settings', { method: 'PUT', body: JSON.stringify(next) })
  },
}

export function friendlyError(error: unknown): string {
  if (error instanceof ApiError) return error.message
  return 'Something went wrong — please try again.'
}
