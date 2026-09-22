import { apiFetch, ApiError } from '../lib/api'
import { registerInvoiceResolver, type Invoice } from '../lib/invoices'
import { isRemoteMail } from './remote-mail'

export type PlanView = {
  code: string
  name: string
  price_cents: number
  extra_mailbox_price_cents: number
  price: string
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
  feature_flags: Record<string, boolean>
  active: boolean
}

export type InvoiceSnapshot = Record<string, string>

export type OrderRow = {
  id: string
  user_id: string
  organization_id: string
  organization_name: string
  email: string
  display_name: string
  plan_code: string
  plan_name: string
  amount_cents: number
  currency: string
  interval: string
  seats: number
  mailbox_count: number
  included_mailbox_count: number
  extra_mailbox_count: number
  extra_mailbox_unit_price_cents: number
  base_price_cents: number
  status: 'pending' | 'submitted' | 'paid' | 'cancelled' | 'rejected'
  payment_method: string
  payment_reference: string
  customer_note: string
  admin_note: string
  invoice_number: string | null
  invoice_status: 'issued' | 'paid' | 'void'
  subtotal_cents: number
  tax_rate_bps: number
  tax_cents: number
  total_cents: number
  seller_snapshot: InvoiceSnapshot
  buyer_snapshot: InvoiceSnapshot
  activation_mode: 'test_instant' | 'payment_approval'
  subscription_assigned_at: string | null
  subscription_assigned_by: string | null
  issued_at: string | null
  due_at: string | null
  period_start: string | null
  period_end: string | null
  activated_at: string | null
  created_at: string
  submitted_at: string | null
  paid_at: string | null
}

export type BusinessSubscriptionRow = {
  organization_id: string
  organization_name: string
  is_system: boolean
  organization_created_at: string
  plan_code: string
  plan_name: string
  status: 'trial' | 'active' | 'past_due' | 'suspended' | 'cancelled'
  purchased_mailbox_count: number
  assignment_source: 'bootstrap' | 'test_instant' | 'payment_approval' | 'admin_manual'
  assigned_at: string | null
  assignment_invoice_number: string | null
  assignment_order_user_email: string | null
  assigned_by_email: string | null
  last_order_id: string | null
  current_period_start: string
  current_period_end: string | null
  renewal_grace_end: string | null
  payment_due_at: string | null
  grace_period_end: string | null
  cancelled_at: string | null
  owner_email: string | null
  billing_email: string | null
  last_paid_at: string | null
  paid_invoice_count: number
  total_paid_cents: number
  storage_allocated_bytes: number
  storage_pool_bytes: number
  usage: { seats: number; mailboxes: number; domains: number; storage_bytes: number }
}

export type BusinessSubscriptionsPage = {
  subscriptions: BusinessSubscriptionRow[]
  total: number
  limit: number
  offset: number
}

export type SubscriptionHistoryRow = {
  id: string
  assigned_at: string
  event_type: 'assignment' | 'status' | 'period' | 'limits'
  assignment_source: 'bootstrap' | 'test_instant' | 'payment_approval' | 'admin_manual' | 'system_lifecycle'
  payment_confirmed: boolean
  plan_code: string
  plan_name: string
  purchased_mailbox_count: number
  invoice_number: string | null
  order_user_email: string | null
  assigned_by_email: string | null
  period_start: string | null
  period_end: string | null
  status_after: BusinessSubscriptionRow['status'] | null
  reason: string
  detail: Record<string, unknown>
}

export type BillingSettingsRow = {
  bank_details: string
  paypal_email: string
  instructions: string
  seller_legal_name: string
  seller_email: string
  seller_cr_number: string
  seller_vat_number: string
  seller_address: string
  tax_rate_bps: number
  invoice_due_days: number
  grace_days: number
}

export type BillingProfileRow = {
  organization_id: string
  legal_name: string
  billing_email: string
  vat_number: string
  cr_number: string
  address_line1: string
  address_line2: string
  city: string
  postal_code: string
  country: string
}

export type BillingSummary = {
  organization_id: string
  subscription_status: string
  current_plan: PlanView
  quota_bytes: number
  mailbox_quota_bytes: number
  storage_pool_bytes: number
  storage_allocated_bytes: number
  seat_limit: number
  mailbox_limit: number
  max_mailboxes: number
  alias_limit_per_mailbox: number | null
  domain_limit: number
  organization_daily_send_limit: number
  usage: { seats: number; mailboxes: number; domains: number }
  quota_override_bytes: number | null
  quota_source: 'plan' | 'override'
  settings: BillingSettingsRow
  billing_profile: BillingProfileRow
  instant_activation: boolean
  orders: OrderRow[]
}

export const paymentMethodLabel = (method: string) =>
  ({ bank: 'Bank transfer', paypal: 'PayPal', card: 'Card', other: 'Other' })[method] ?? method

export function formatPrice(cents: number, currency: string): string {
  const code = currency.toUpperCase()
  if (code === 'SAR') return `SAR ${(cents / 100).toFixed(2)}`
  try {
    return new Intl.NumberFormat(undefined, { style: 'currency', currency: code }).format(cents / 100)
  } catch {
    return `${code} ${(cents / 100).toFixed(2)}`
  }
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

const featureFlags = { mail: true, attachments: true, scheduled_send: true, read_receipts: true, contacts: true, calendar: true }
const DEMO_PLANS: PlanView[] = [
  { code:'solo', name:'CS Mail Start', price_cents:5900, extra_mailbox_price_cents:3500, price:'SAR 59.00', currency:'SAR', interval:'year', mailbox_bytes:5*1024**3, storage_pool_bytes:5*1024**3, mailbox_limit:1, max_mailboxes:50, alias_limit_per_mailbox:10, domain_limit:1, organization_daily_send_limit:2000, max_attachment_bytes:25*1024**2, max_recipients:50, daily_send_limit:2000, seats:1, features:['1 mailbox included','5 GB storage per mailbox','10 aliases per mailbox','1 custom domain','Webmail + IMAP/SMTP','Additional mailboxes available'], feature_flags:featureFlags, active:true },
  { code:'team', name:'CS Mail Grow', price_cents:15900, extra_mailbox_price_cents:9900, price:'SAR 159.00', currency:'SAR', interval:'year', mailbox_bytes:10*1024**3, storage_pool_bytes:30*1024**3, mailbox_limit:3, max_mailboxes:50, alias_limit_per_mailbox:50, domain_limit:3, organization_daily_send_limit:6000, max_attachment_bytes:50*1024**2, max_recipients:50, daily_send_limit:2000, seats:3, features:['3 mailboxes included','10 GB storage per mailbox','50 aliases per mailbox','Up to 3 custom domains','Webmail + IMAP/SMTP','Additional mailboxes available'], feature_flags:featureFlags, active:true },
  { code:'business', name:'CS Mail Scale', price_cents:26900, extra_mailbox_price_cents:14900, price:'SAR 269.00', currency:'SAR', interval:'year', mailbox_bytes:15*1024**3, storage_pool_bytes:75*1024**3, mailbox_limit:5, max_mailboxes:50, alias_limit_per_mailbox:null, domain_limit:5, organization_daily_send_limit:10000, max_attachment_bytes:100*1024**2, max_recipients:50, daily_send_limit:2000, seats:5, features:['5 mailboxes included','15 GB storage per mailbox','Unlimited aliases per mailbox','Up to 5 custom domains','Webmail + IMAP/SMTP','Additional mailboxes available'], feature_flags:featureFlags, active:true },
]

const DEMO_SETTINGS: BillingSettingsRow = {
  bank_details: 'Configure your business bank-transfer instructions in Admin → Billing.',
  paypal_email: '',
  instructions: 'Pay the issued invoice and submit the transfer/reference from Billing. During testing, ordering activates the plan immediately.',
  seller_legal_name: 'CrescentSphere',
  seller_email: 'billing@crescentsphere.com',
  seller_cr_number: '', seller_vat_number: '', seller_address: '', tax_rate_bps: 1500, invoice_due_days: 7, grace_days: 7,
}
const DEMO_PROFILE: BillingProfileRow = { organization_id:'demo-business', legal_name:'Demo business', billing_email:'you@example.com', vat_number:'', cr_number:'', address_line1:'', address_line2:'', city:'', postal_code:'', country:'Saudi Arabia' }

const nowIso = () => new Date().toISOString()
const plusDays = (days:number) => new Date(Date.now()+days*86400000).toISOString()
let demoOrders: OrderRow[] = []
let demoPlans: PlanView[] = [...DEMO_PLANS]
let demoSettings: BillingSettingsRow = { ...DEMO_SETTINGS }
let demoProfile: BillingProfileRow = { ...DEMO_PROFILE }

function demoOrder(plan: PlanView, mailboxCount: number, method: string, note: string): OrderRow {
  const quantity = Math.max(plan.mailbox_limit, Math.min(plan.max_mailboxes, mailboxCount))
  const extraCount = Math.max(0, quantity - plan.mailbox_limit)
  const subtotal = plan.price_cents + extraCount * plan.extra_mailbox_price_cents
  const taxBps = demoSettings.seller_vat_number ? demoSettings.tax_rate_bps : 0
  const tax = Math.round(subtotal * taxBps / 10000)
  const total = subtotal + tax
  const now=nowIso()
  const invoice=`INV-${new Date().getFullYear()}-${String(Date.now()).slice(-6)}`
  const periodEnd = new Date()
  periodEnd.setFullYear(periodEnd.getFullYear()+1)
  return { id:`demo-${Date.now()}`, user_id:'demo', organization_id:'demo-business', organization_name:demoProfile.legal_name || 'Demo business', email:'you@example.com', display_name:'You', plan_code:plan.code, plan_name:plan.name, amount_cents:total, currency:plan.currency, interval:plan.interval, seats:quantity, mailbox_count:quantity, included_mailbox_count:plan.mailbox_limit, extra_mailbox_count:extraCount, extra_mailbox_unit_price_cents:plan.extra_mailbox_price_cents, base_price_cents:plan.price_cents, status:'pending', payment_method:method, payment_reference:'', customer_note:note, admin_note:'', invoice_number:invoice, invoice_status:'issued', subtotal_cents:subtotal, tax_rate_bps:taxBps, tax_cents:tax, total_cents:total, seller_snapshot:{ legal_name:demoSettings.seller_legal_name, email:demoSettings.seller_email, vat_number:demoSettings.seller_vat_number, cr_number:demoSettings.seller_cr_number, address:demoSettings.seller_address }, buyer_snapshot:{ legal_name:demoProfile.legal_name, billing_email:demoProfile.billing_email, vat_number:demoProfile.vat_number, cr_number:demoProfile.cr_number, address_line1:demoProfile.address_line1, city:demoProfile.city, postal_code:demoProfile.postal_code, country:demoProfile.country }, activation_mode:'test_instant', subscription_assigned_at:now, subscription_assigned_by:null, issued_at:now, due_at:plusDays(demoSettings.invoice_due_days), period_start:now, period_end:periodEnd.toISOString(), activated_at:now, created_at:now, submitted_at:null, paid_at:null }
}

const snapshotAddress = (snapshot: InvoiceSnapshot) => [snapshot.address_line1, snapshot.address_line2, snapshot.city, snapshot.postal_code, snapshot.country].filter(Boolean).join(', ')

export function invoiceFromOrder(order: OrderRow): Invoice | null {
  if (!order.invoice_number) return null
  const issued = new Date(order.issued_at ?? order.created_at)
  const due = new Date(order.due_at ?? order.created_at)
  const periodStart = new Date(order.period_start ?? order.issued_at ?? order.created_at)
  const periodEnd = new Date(order.period_end ?? order.due_at ?? order.created_at)
  return {
    id: order.invoice_number,
    period: `${periodStart.toLocaleDateString()} – ${periodEnd.toLocaleDateString()}`,
    date: issued.toLocaleDateString([], { month:'long', day:'numeric', year:'numeric' }),
    dueDate: due.toLocaleDateString([], { month:'long', day:'numeric', year:'numeric' }),
    amount: formatPrice(order.total_cents,order.currency),
    status: order.invoice_status,
    plan: order.plan_name,
    mailboxCount: order.mailbox_count,
    includedMailboxCount: order.included_mailbox_count,
    extraMailboxCount: order.extra_mailbox_count,
    extraMailboxUnitPrice: formatPrice(order.extra_mailbox_unit_price_cents,order.currency),
    extraMailboxTotal: formatPrice(order.extra_mailbox_count * order.extra_mailbox_unit_price_cents,order.currency),
    baseRate: formatPrice(order.base_price_cents,order.currency),
    rate: formatPrice(order.subtotal_cents,order.currency),
    subtotal: formatPrice(order.subtotal_cents,order.currency),
    tax: formatPrice(order.tax_cents,order.currency),
    taxRate: `${(order.tax_rate_bps/100).toFixed(2)}%`,
    total: formatPrice(order.total_cents,order.currency),
    paymentMethod: paymentMethodLabel(order.payment_method),
    billedTo: order.buyer_snapshot.billing_email || order.email,
    sellerName: order.seller_snapshot.legal_name || 'CrescentSphere',
    sellerVat: order.seller_snapshot.vat_number || '', sellerCr:order.seller_snapshot.cr_number || '', sellerAddress:order.seller_snapshot.address || '',
    buyerName:order.buyer_snapshot.legal_name || order.organization_name, buyerVat:order.buyer_snapshot.vat_number || '', buyerCr:order.buyer_snapshot.cr_number || '', buyerAddress:snapshotAddress(order.buyer_snapshot),
  }
}

export const billingApi = {
  async summary(): Promise<BillingSummary> {
    if (!isRemoteMail()) { const plan=demoPlans[1] ?? demoPlans[0]; return { organization_id:'demo-business',subscription_status:'active',current_plan:plan,quota_bytes:plan.mailbox_bytes,mailbox_quota_bytes:plan.mailbox_bytes,storage_pool_bytes:plan.mailbox_bytes*plan.mailbox_limit,storage_allocated_bytes:plan.mailbox_bytes,seat_limit:plan.mailbox_limit,mailbox_limit:plan.mailbox_limit,max_mailboxes:plan.max_mailboxes,alias_limit_per_mailbox:plan.alias_limit_per_mailbox,domain_limit:plan.domain_limit,organization_daily_send_limit:plan.organization_daily_send_limit,usage:{seats:1,mailboxes:1,domains:1},quota_override_bytes:null,quota_source:'plan',settings:demoSettings,billing_profile:demoProfile,instant_activation:true,orders:demoOrders } }
    return apiFetch<BillingSummary>('/api/billing')
  },
  async plans(): Promise<PlanView[]> { if(!isRemoteMail()) return demoPlans; return (await apiFetch<{plans:PlanView[]}>('/api/billing/plans')).plans },
  async createOrder(planCode:string,mailboxCount:number,paymentMethod:string,customerNote:string):Promise<OrderRow> { if(!isRemoteMail()){const plan=demoPlans.find(p=>p.code===planCode)??demoPlans[0];const order=demoOrder(plan,mailboxCount,paymentMethod,customerNote);demoOrders=[order,...demoOrders];return order} return apiFetch<OrderRow>('/api/billing/orders',{method:'POST',body:JSON.stringify({plan_code:planCode,mailbox_count:mailboxCount,payment_method:paymentMethod,customer_note:customerNote})}) },
  async submitPaid(orderId:string,paymentMethod:string,reference:string):Promise<OrderRow>{ if(!isRemoteMail()){demoOrders=demoOrders.map(o=>o.id===orderId?{...o,status:'submitted',payment_reference:reference,submitted_at:nowIso()}:o);return demoOrders.find(o=>o.id===orderId)!} return apiFetch<OrderRow>(`/api/billing/orders/${encodeURIComponent(orderId)}/paid`,{method:'POST',body:JSON.stringify({payment_method:paymentMethod,payment_reference:reference})}) },
  async cancelOrder(orderId:string):Promise<void>{ if(!isRemoteMail()){demoOrders=demoOrders.map(o=>o.id===orderId?{...o,status:'cancelled',invoice_status:'void'}:o);return} await apiFetch(`/api/billing/orders/${encodeURIComponent(orderId)}/cancel`,{method:'POST'}) },
  async invoices():Promise<OrderRow[]>{ const summary=await billingApi.summary(); return summary.orders.filter(o=>Boolean(o.invoice_number)) },
  async updateProfile(profile:BillingProfileRow):Promise<BillingProfileRow>{ if(!isRemoteMail()){demoProfile={...profile};return demoProfile} return apiFetch<BillingProfileRow>('/api/billing/profile',{method:'PUT',body:JSON.stringify(profile)}) },
  refreshInvoices(){ void billingApi.invoices().then(registerInvoiceResolverCached) },
}

let cachedInvoices: Invoice[]=[]
function registerInvoiceResolverCached(rows:OrderRow[]){ cachedInvoices=rows.map(invoiceFromOrder).filter((invoice):invoice is Invoice=>invoice!==null); registerInvoiceResolver(id=>cachedInvoices.find(invoice=>invoice.id.toLowerCase()===id.toLowerCase())) }

export const adminBillingApi = {
  async plans():Promise<PlanView[]>{ if(!isRemoteMail()) return demoPlans; return (await apiFetch<{plans:PlanView[]}>('/api/admin/plans')).plans },
  async createPlan(input:PlanView):Promise<void>{ if(!isRemoteMail()){demoPlans=[...demoPlans,{...input,code:input.code.toLowerCase().replace(/\s+/g,'-')}];return} await apiFetch('/api/admin/plans',{method:'POST',body:JSON.stringify(input)}) },
  async updatePlan(code:string,input:PlanView):Promise<void>{ if(!isRemoteMail()){demoPlans=demoPlans.map(plan=>plan.code===code?{...input,code}:plan);return} await apiFetch(`/api/admin/plans/${encodeURIComponent(code)}`,{method:'PATCH',body:JSON.stringify(input)}) },
  async deactivatePlan(code:string):Promise<void>{ if(!isRemoteMail()){demoPlans=demoPlans.map(plan=>plan.code===code?{...plan,active:false}:plan);return} await apiFetch(`/api/admin/plans/${encodeURIComponent(code)}`,{method:'DELETE'}) },
  async orders(status?:string):Promise<OrderRow[]>{ if(!isRemoteMail()) return demoOrders; const query=status&&status!=='queue'?`?status=${encodeURIComponent(status)}`:''; return (await apiFetch<{orders:OrderRow[]}>(`/api/admin/orders${query}`)).orders },
  async approveOrder(orderId:string,adminNote:string):Promise<void>{ if(!isRemoteMail()){demoOrders=demoOrders.map(order=>order.id===orderId?{...order,status:'paid',invoice_status:'paid',admin_note:adminNote,paid_at:nowIso()}:order);return} await apiFetch(`/api/admin/orders/${encodeURIComponent(orderId)}/approve`,{method:'POST',body:JSON.stringify({admin_note:adminNote})}) },
  async rejectOrder(orderId:string,adminNote:string):Promise<void>{ if(!isRemoteMail()){demoOrders=demoOrders.map(order=>order.id===orderId?{...order,status:'rejected',invoice_status:'void',admin_note:adminNote}:order);return} await apiFetch(`/api/admin/orders/${encodeURIComponent(orderId)}/reject`,{method:'POST',body:JSON.stringify({admin_note:adminNote})}) },
  async subscriptionsPage(options:{q?:string;status?:BusinessSubscriptionRow['status'];plan?:string;organizationId?:string;limit?:number;offset?:number}={}):Promise<BusinessSubscriptionsPage>{
    if(!isRemoteMail()){const plan=demoPlans[1]??demoPlans[0];const now=nowIso();const subscriptions=[{organization_id:'demo-business',organization_name:'Demo business',is_system:false,organization_created_at:now,plan_code:plan.code,plan_name:plan.name,status:'active' as const,purchased_mailbox_count:plan.mailbox_limit,assignment_source:'test_instant' as const,assigned_at:now,assignment_invoice_number:null,assignment_order_user_email:'you@example.com',assigned_by_email:null,last_order_id:null,current_period_start:now,current_period_end:new Date(Date.now()+365*86400000).toISOString(),renewal_grace_end:null,payment_due_at:null,grace_period_end:null,cancelled_at:null,owner_email:'you@example.com',billing_email:'you@example.com',last_paid_at:null,paid_invoice_count:0,total_paid_cents:0,storage_allocated_bytes:plan.mailbox_bytes,storage_pool_bytes:plan.storage_pool_bytes,usage:{seats:1,mailboxes:1,domains:1,storage_bytes:0}}];return {subscriptions,total:subscriptions.length,limit:options.limit??100,offset:options.offset??0}}
    const params=new URLSearchParams(); if(options.q?.trim())params.set('q',options.q.trim()); if(options.status)params.set('status',options.status); if(options.plan)params.set('plan',options.plan); if(options.organizationId)params.set('organization_id',options.organizationId); params.set('limit',String(options.limit??100)); params.set('offset',String(options.offset??0));
    const data=await apiFetch<{subscriptions:BusinessSubscriptionRow[];total?:number;limit?:number;offset?:number}>(`/api/admin/subscriptions?${params.toString()}`); return {subscriptions:data.subscriptions??[],total:data.total??data.subscriptions?.length??0,limit:data.limit??options.limit??100,offset:data.offset??options.offset??0}
  },
  async subscriptions():Promise<BusinessSubscriptionRow[]>{ return (await this.subscriptionsPage()).subscriptions },
  async subscriptionHistory(organizationId:string):Promise<SubscriptionHistoryRow[]>{ if(!isRemoteMail()) return []; return (await apiFetch<{history:SubscriptionHistoryRow[]}>(`/api/admin/subscriptions/${encodeURIComponent(organizationId)}/history`)).history },
  async updateSubscription(organizationId:string,input:{plan_code:string;status:BusinessSubscriptionRow['status'];purchased_mailbox_count?:number;current_period_end?:string;reason?:string}):Promise<void>{ if(!isRemoteMail()) return; await apiFetch(`/api/admin/subscriptions/${encodeURIComponent(organizationId)}`,{method:'PATCH',body:JSON.stringify(input)}) },
  async settings():Promise<BillingSettingsRow>{ if(!isRemoteMail()) return demoSettings; return apiFetch<BillingSettingsRow>('/api/admin/billing-settings') },
  async updateSettings(next:BillingSettingsRow):Promise<void>{ if(!isRemoteMail()){demoSettings={...next};return} await apiFetch('/api/admin/billing-settings',{method:'PUT',body:JSON.stringify(next)}) },
}

export function friendlyError(error:unknown):string{ if(error instanceof ApiError) return error.message; return 'Something went wrong — please try again.' }
