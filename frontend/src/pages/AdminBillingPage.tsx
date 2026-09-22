import { useEffect, useState, type FormEvent } from 'react'
import { useNavigate, useSearchParams } from 'react-router-dom'
import { ArrowLeft, Check, CreditCard, History, Landmark, Plus, Save, Trash2 } from 'lucide-react'
import {
  adminBillingApi,
  billingApi,
  formatPrice,
  friendlyError,
  paymentMethodLabel,
  splitBytes,
  type BillingSettingsRow,
  type BusinessSubscriptionRow,
  type OrderRow,
  type PlanView,
  type SubscriptionHistoryRow,
} from '../services/billing'

type AdminTab = 'orders' | 'subscriptions' | 'plans' | 'settings'

const entitlementFeatures = [
  ['mail', 'Mailbox access'],
  ['attachments', 'Attachments'],
  ['scheduled_send', 'Scheduled send'],
  ['read_receipts', 'Read receipts'],
  ['contacts', 'Contacts'],
  ['calendar', 'Calendar'],
] as const

const statusLabel: Record<OrderRow['status'], string> = {
  pending: 'Awaiting payment',
  submitted: 'Verifying payment',
  paid: 'Paid',
  cancelled: 'Cancelled',
  rejected: 'Rejected',
}

const emptyPlan = (): PlanView => ({
  code: '',
  name: '',
  price_cents: 0,
  extra_mailbox_price_cents: 0,
  price: 'SAR 0.00',
  currency: 'SAR',
  interval: 'year',
  mailbox_bytes: 5 * 1024 * 1024 * 1024,
  storage_pool_bytes: 5 * 1024 * 1024 * 1024,
  mailbox_limit: 1,
  max_mailboxes: 50,
  alias_limit_per_mailbox: 10,
  domain_limit: 1,
  organization_daily_send_limit: 500,
  max_attachment_bytes: 25 * 1024 * 1024,
  max_recipients: 30,
  daily_send_limit: 500,
  seats: 1,
  features: [],
  feature_flags: { mail: true, attachments: true, scheduled_send: true, read_receipts: true, contacts: true, calendar: true },
  active: true,
})

export function AdminBillingPage() {
  const navigate = useNavigate()
  const [searchParams] = useSearchParams()
  const focusedBusinessId = searchParams.get('business')
  const [tab, setTab] = useState<AdminTab>('orders')
  const [orders, setOrders] = useState<OrderRow[]>([])
  const [plans, setPlans] = useState<PlanView[]>([])
  const [subscriptions, setSubscriptions] = useState<BusinessSubscriptionRow[]>([])
  const subscriptionPageSize = 100
  const [subscriptionPage, setSubscriptionPage] = useState(0)
  const [subscriptionTotal, setSubscriptionTotal] = useState(0)
  const [subscriptionQuery, setSubscriptionQuery] = useState('')
  const [subscriptionStatus, setSubscriptionStatus] = useState<'all' | BusinessSubscriptionRow['status']>('all')
  const [subscriptionPlan, setSubscriptionPlan] = useState('all')
  const [historyByOrg, setHistoryByOrg] = useState<Record<string, SubscriptionHistoryRow[]>>({})
  const [openHistoryOrg, setOpenHistoryOrg] = useState<string | null>(null)
  const [settings, setSettings] = useState<BillingSettingsRow>({
    bank_details: '', paypal_email: '', instructions: '',
    seller_legal_name: 'CrescentSphere', seller_email: 'billing@crescentsphere.com',
    seller_cr_number: '', seller_vat_number: '', seller_address: '',
    tax_rate_bps: 1500, invoice_due_days: 7, grace_days: 7,
  })
  const [filter, setFilter] = useState('queue')
  const [reviewId, setReviewId] = useState<string | null>(null)
  const [reviewNote, setReviewNote] = useState('')
  const [editing, setEditing] = useState<PlanView | null>(null)
  const [planForm, setPlanForm] = useState<PlanView>(emptyPlan())
  const [busy, setBusy] = useState(false)
  const [notice, setNotice] = useState('')

  const reloadOrders = async () => {
    const next = await adminBillingApi.orders(filter)
    setOrders(next)
  }
  const reloadPlans = async () => {
    setPlans(await adminBillingApi.plans())
  }
  const reloadSubscriptions = async (page = subscriptionPage) => {
    const result = await adminBillingApi.subscriptionsPage({
      q: focusedBusinessId ? undefined : subscriptionQuery,
      status: focusedBusinessId || subscriptionStatus === 'all' ? undefined : subscriptionStatus,
      plan: focusedBusinessId || subscriptionPlan === 'all' ? undefined : subscriptionPlan,
      organizationId: focusedBusinessId ?? undefined,
      limit: focusedBusinessId ? 1 : subscriptionPageSize,
      offset: focusedBusinessId ? 0 : page * subscriptionPageSize,
    })
    setSubscriptions(result.subscriptions)
    setSubscriptionTotal(result.total)
  }

  useEffect(() => {
    void adminBillingApi.orders(filter).then(setOrders)
  }, [filter])

  useEffect(() => {
    void adminBillingApi.plans().then(setPlans)
    void adminBillingApi
      .settings()
      .then(setSettings)
      .catch(() => {})
  }, [])

  useEffect(() => {
    if (focusedBusinessId) {
      setTab('subscriptions')
      setSubscriptionPage(0)
    }
  }, [focusedBusinessId])

  useEffect(() => {
    const timer = window.setTimeout(() => {
      void reloadSubscriptions(focusedBusinessId ? 0 : subscriptionPage).catch((error) => showNotice(friendlyError(error)))
    }, 200)
    return () => window.clearTimeout(timer)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [focusedBusinessId, subscriptionQuery, subscriptionStatus, subscriptionPlan, subscriptionPage])

  const showNotice = (message: string) => {
    setNotice(message)
    window.setTimeout(() => setNotice(''), 5000)
  }

  const applySubscription = async (
    subscription: BusinessSubscriptionRow,
    patch: Partial<{ plan_code: string; status: BusinessSubscriptionRow['status']; purchased_mailbox_count: number; current_period_end: string; reason: string }>,
    message: string,
  ) => {
    setBusy(true)
    try {
      await adminBillingApi.updateSubscription(subscription.organization_id, {
        plan_code: patch.plan_code ?? subscription.plan_code,
        status: patch.status ?? subscription.status,
        purchased_mailbox_count: patch.purchased_mailbox_count ?? subscription.purchased_mailbox_count,
        current_period_end: patch.current_period_end,
        reason: patch.reason,
      })
      await reloadSubscriptions()
      if (openHistoryOrg === subscription.organization_id) {
        const history = await adminBillingApi.subscriptionHistory(subscription.organization_id)
        setHistoryByOrg((current) => ({ ...current, [subscription.organization_id]: history }))
      }
      showNotice(message)
    } catch (error) {
      showNotice(friendlyError(error))
    } finally {
      setBusy(false)
    }
  }

  const toggleHistory = async (organizationId: string) => {
    if (openHistoryOrg === organizationId) {
      setOpenHistoryOrg(null)
      return
    }
    setOpenHistoryOrg(organizationId)
    try {
      const history = await adminBillingApi.subscriptionHistory(organizationId)
      setHistoryByOrg((current) => ({ ...current, [organizationId]: history }))
    } catch (error) {
      showNotice(friendlyError(error))
    }
  }

  const approveOrder = async (order: OrderRow) => {
    setBusy(true)
    try {
      await adminBillingApi.approveOrder(order.id, reviewNote.trim())
      setReviewId(null)
      setReviewNote('')
      showNotice(`${order.plan_name} · ${order.mailbox_count} mailbox${order.mailbox_count === 1 ? '' : 'es'} assigned from ${order.invoice_number ?? 'the paid invoice'}.`)
      await reloadOrders()
      await reloadPlans()
      billingApi.refreshInvoices()
    } catch (error) {
      showNotice(friendlyError(error))
    } finally {
      setBusy(false)
    }
  }

  const rejectOrder = async (order: OrderRow) => {
    setBusy(true)
    try {
      await adminBillingApi.rejectOrder(order.id, reviewNote.trim())
      setReviewId(null)
      setReviewNote('')
      showNotice(`Order for ${order.email} rejected.`)
      await reloadOrders()
    } catch (error) {
      showNotice(friendlyError(error))
    } finally {
      setBusy(false)
    }
  }

  const startEdit = (plan: PlanView) => {
    setEditing(plan)
    setPlanForm({ ...plan })
  }
  const startCreate = () => {
    setEditing(null)
    setPlanForm(emptyPlan())
  }

  const savePlan = async (event: FormEvent) => {
    event.preventDefault()
    const input: PlanView = {
      ...planForm,
      code: planForm.code.trim().toLowerCase().replace(/\s+/g, '-'),
      name: planForm.name.trim(),
      currency: (planForm.currency || 'SAR').toUpperCase(),
      interval: planForm.interval === 'year' ? 'year' : 'month',
      price_cents: Math.max(0, planForm.price_cents || 0),
      extra_mailbox_price_cents: Math.max(0, planForm.extra_mailbox_price_cents || 0),
      mailbox_bytes: Math.max(1, planForm.mailbox_bytes || 1),
      storage_pool_bytes: Math.max(1, planForm.storage_pool_bytes || planForm.mailbox_bytes || 1),
      mailbox_limit: Math.max(1, planForm.mailbox_limit || planForm.seats || 1),
      max_mailboxes: Math.max(planForm.mailbox_limit || 1, planForm.max_mailboxes || planForm.mailbox_limit || 1),
      alias_limit_per_mailbox: planForm.alias_limit_per_mailbox && planForm.alias_limit_per_mailbox > 0 ? planForm.alias_limit_per_mailbox : null,
      domain_limit: Math.max(1, planForm.domain_limit || 1),
      organization_daily_send_limit: Math.max(0, planForm.organization_daily_send_limit || planForm.daily_send_limit || 0),
      max_attachment_bytes: Math.max(1, planForm.max_attachment_bytes || 1),
      max_recipients: Math.max(1, planForm.max_recipients || 1),
      daily_send_limit: Math.max(0, planForm.daily_send_limit || 0),
      seats: Math.max(1, planForm.seats || 1),
      features: (planForm.features ?? [])
        .filter((feature) => feature.trim())
        .map((feature) => feature.trim()),
    }
    if (!input.code || !input.name) {
      showNotice('Plan code and name are required.')
      return
    }
    setBusy(true)
    try {
      if (editing) {
        await adminBillingApi.updatePlan(editing.code, input)
        showNotice(`Plan ${input.name} updated.`)
      } else {
        await adminBillingApi.createPlan(input)
        showNotice(`Plan ${input.name} created.`)
      }
      setEditing(null)
      setPlanForm(emptyPlan())
      await reloadPlans()
    } catch (error) {
      showNotice(friendlyError(error))
    } finally {
      setBusy(false)
    }
  }

  const deactivatePlan = async (plan: PlanView) => {
    setBusy(true)
    try {
      await adminBillingApi.deactivatePlan(plan.code)
      showNotice(`Plan ${plan.name} deactivated — existing subscribers keep theirs.`)
      await reloadPlans()
    } catch (error) {
      showNotice(friendlyError(error))
    } finally {
      setBusy(false)
    }
  }

  const saveSettings = async (event: FormEvent) => {
    event.preventDefault()
    setBusy(true)
    try {
      await adminBillingApi.updateSettings(settings)
      showNotice('Payment instructions updated.')
    } catch (error) {
      showNotice(friendlyError(error))
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="settings-page" role="region" aria-label="Admin billing">
      <header className="calendar-head">
        <div>
          <p className="eyebrow">Workspace / Admin / Billing</p>
          <h1>Payments &amp; plans</h1>
        </div>
        <div className="calendar-head__actions">
          <button
            type="button"
            className="secondary-button"
            onClick={() => navigate('/mail/admin')}
          >
            <ArrowLeft size={14} />
            Admin center
          </button>
        </div>
      </header>

      <nav className="admin-nav" aria-label="Billing admin sections">
        <button
          type="button"
          className={tab === 'orders' ? 'admin-nav--active' : ''}
          aria-current={tab === 'orders' ? 'page' : undefined}
          onClick={() => setTab('orders')}
        >
          <Landmark size={14} />
          Orders
        </button>
        <button
          type="button"
          className={tab === 'subscriptions' ? 'admin-nav--active' : ''}
          aria-current={tab === 'subscriptions' ? 'page' : undefined}
          onClick={() => setTab('subscriptions')}
        >
          <Landmark size={14} />
          Businesses
        </button>
        <button
          type="button"
          className={tab === 'plans' ? 'admin-nav--active' : ''}
          aria-current={tab === 'plans' ? 'page' : undefined}
          onClick={() => setTab('plans')}
        >
          <CreditCard size={14} />
          Plans
        </button>
        <button
          type="button"
          className={tab === 'settings' ? 'admin-nav--active' : ''}
          aria-current={tab === 'settings' ? 'page' : undefined}
          onClick={() => setTab('settings')}
        >
          <Save size={14} />
          Settings
        </button>
      </nav>

      {notice && <p className="settings-notice settings-notice--ok">{notice}</p>}

      {tab === 'orders' && (
        <section className="settings-section">
          <div className="admin-section-head">
            <h2>Order queue</h2>
            <select
              value={filter}
              onChange={(event) => setFilter(event.target.value)}
              aria-label="Filter orders by status"
            >
              <option value="queue">Open queue</option>
              <option value="paid">Paid</option>
              <option value="cancelled">Cancelled</option>
              <option value="rejected">Rejected</option>
            </select>
          </div>
          {orders.length === 0 && (
            <p className="settings-hint">
              Nothing here. Orders appear when a customer places (and submits) a payment.
            </p>
          )}
          {orders.map((order) => (
            <div className="billing-row" key={order.id}>
              <div>
                <strong>
                  {order.display_name || order.email} · {order.organization_name} · {order.plan_name} ·{' '}
                  {order.mailbox_count} mailbox{order.mailbox_count === 1 ? '' : 'es'} · {formatPrice(order.amount_cents, order.currency)}
                </strong>
                <small>
                  {paymentMethodLabel(order.payment_method)}
                  {order.payment_reference ? ` · ${order.payment_reference}` : ''} · created{' '}
                  {new Date(order.created_at).toLocaleString([], {
                    month: 'short',
                    day: 'numeric',
                    hour: 'numeric',
                    minute: '2-digit',
                  })}
                  {order.customer_note ? ` · note: ${order.customer_note}` : ''}
                  {order.invoice_number ? ` · ${order.invoice_number}` : ''}
                  {order.admin_note ? ` · ${order.admin_note}` : ''}{order.subscription_assigned_at ? ` · plan assigned ${new Date(order.subscription_assigned_at).toLocaleString()}` : ''}
                </small>
              </div>
              <div className="admin-actions">
                <span className={order.status === 'paid' ? 'billing-paid' : ''}>
                  {order.status === 'paid' && <Check size={13} />}
                  {statusLabel[order.status]}
                </span>
                {reviewId === order.id ? (
                  <div className="admin-inline-form">
                    <input
                      value={reviewNote}
                      onChange={(event) => setReviewNote(event.target.value)}
                      placeholder="Admin note for the customer"
                      aria-label="Admin note"
                    />
                    {order.status === 'submitted' && (
                      <button
                        type="button"
                        className="primary-button"
                        disabled={busy}
                        onClick={() => void approveOrder(order)}
                      >
                        Approve
                      </button>
                    )}
                    <button
                      type="button"
                      className="secondary-button"
                      disabled={busy}
                      onClick={() => void rejectOrder(order)}
                    >
                      Reject
                    </button>
                  </div>
                ) : (
                  (order.status === 'pending' || order.status === 'submitted') && (
                    <button
                      type="button"
                      className="secondary-button"
                      onClick={() => {
                        setReviewId(order.id)
                        setReviewNote('')
                      }}
                    >
                      Review
                    </button>
                  )
                )}
              </div>
            </div>
          ))}
        </section>
      )}

      {tab === 'subscriptions' && (
        <section className="settings-section">
          <div className="admin-section-head">
            <h2>Business subscriptions</h2>
            <button type="button" className="secondary-button" onClick={() => void reloadSubscriptions().catch((error) => showNotice(friendlyError(error)))}>Refresh</button>
          </div>
          <p className="settings-hint">Authoritative platform view of who has each plan, how it was assigned, when it became active, when it expires, and its payment history. Manual changes are written to the subscription history ledger.</p>
          {focusedBusinessId ? (
            <div className="admin-list-tools">
              <span className="business-badge">Focused business</span>
              <button type="button" className="secondary-button" onClick={() => navigate('/mail/admin/billing')}>Show all businesses</button>
            </div>
          ) : (
            <div className="admin-list-tools">
              <label className="admin-search">
                <input
                  value={subscriptionQuery}
                  onChange={(event) => { setSubscriptionQuery(event.target.value); setSubscriptionPage(0) }}
                  placeholder="Search business, owner, billing email, plan or invoice"
                  aria-label="Search business subscriptions"
                />
              </label>
              <select value={subscriptionStatus} onChange={(event) => { setSubscriptionStatus(event.target.value as typeof subscriptionStatus); setSubscriptionPage(0) }} aria-label="Filter subscriptions by status">
                <option value="all">All statuses</option>
                <option value="trial">Trial</option>
                <option value="active">Active</option>
                <option value="past_due">Past due</option>
                <option value="suspended">Suspended</option>
                <option value="cancelled">Cancelled</option>
              </select>
              <select value={subscriptionPlan} onChange={(event) => { setSubscriptionPlan(event.target.value); setSubscriptionPage(0) }} aria-label="Filter subscriptions by plan">
                <option value="all">All plans</option>
                {plans.map((item) => <option key={item.code} value={item.code}>{item.name}</option>)}
              </select>
              <span className="admin-section-count">{subscriptions.length} of {subscriptionTotal}</span>
            </div>
          )}
          {subscriptions.map((subscription) => {
            const plan = plans.find((item) => item.code === subscription.plan_code)
            const history = historyByOrg[subscription.organization_id] ?? []
            const expiryDate = subscription.current_period_end ? subscription.current_period_end.slice(0, 10) : ''
            return (
              <div className={`admin-subscription-card${subscription.organization_id === focusedBusinessId ? ' admin-subscription-card--focused' : ''}`} key={subscription.organization_id}>
                <div className="billing-row">
                  <div>
                    <strong>{subscription.organization_name}{subscription.is_system ? ' · System' : ''}</strong>
                    <small>
                      {subscription.plan_name} · {subscription.purchased_mailbox_count} purchased · {subscription.usage.mailboxes} provisioned · {subscription.usage.domains} domains
                      {subscription.owner_email ? ` · owner ${subscription.owner_email}` : ''}
                      {subscription.billing_email ? ` · billing ${subscription.billing_email}` : ''}
                    </small>
                    <small>
                      Activated {subscription.assigned_at ? new Date(subscription.assigned_at).toLocaleString() : '—'} via {subscription.assignment_source.replaceAll('_', ' ')}
                      {subscription.assignment_invoice_number ? ` · ${subscription.assignment_invoice_number}` : ''}
                      {subscription.assignment_order_user_email ? ` · ordered by ${subscription.assignment_order_user_email}` : ''}
                      {subscription.assigned_by_email ? ` · assigned by ${subscription.assigned_by_email}` : ''}
                    </small>
                    <small>
                      Period {new Date(subscription.current_period_start).toLocaleDateString()} → {subscription.current_period_end ? new Date(subscription.current_period_end).toLocaleDateString() : 'no expiry'}
                      {subscription.renewal_grace_end ? ` · grace until ${new Date(subscription.renewal_grace_end).toLocaleDateString()}` : ''}
                      {subscription.payment_due_at ? ` · payment due ${new Date(subscription.payment_due_at).toLocaleDateString()}` : ''}
                      {subscription.last_paid_at ? ` · last paid ${new Date(subscription.last_paid_at).toLocaleDateString()}` : ' · no reviewed payment yet'}
                      {` · ${subscription.paid_invoice_count} paid invoice${subscription.paid_invoice_count === 1 ? '' : 's'} · ${formatPrice(subscription.total_paid_cents, plan?.currency ?? 'SAR')} collected`}
                    </small>
                    <small>Business created {new Date(subscription.organization_created_at).toLocaleDateString()}</small>
                    <small>
                      Storage {(subscription.usage.storage_bytes / 1073741824).toFixed(1)} GB used · {(subscription.storage_allocated_bytes / 1073741824).toFixed(1)} GB allocated / {(subscription.storage_pool_bytes / 1073741824).toFixed(1)} GB pool
                    </small>
                  </div>
                  <div className="admin-actions">
                    <span className={`business-badge business-badge--${subscription.status}`}>{subscription.status.replace('_', ' ')}</span>
                    <button type="button" className="secondary-button" onClick={() => void toggleHistory(subscription.organization_id)}>
                      <History size={13} /> History
                    </button>
                  </div>
                </div>
                <div className="admin-subscription-controls">
                  <label>
                    Plan
                    <select
                      aria-label={`Plan for ${subscription.organization_name}`}
                      value={subscription.plan_code}
                      disabled={busy || subscription.is_system}
                      onChange={(event) => {
                        const nextPlan = plans.find((item) => item.code === event.target.value)
                        const nextMailboxCount = Math.max(subscription.purchased_mailbox_count, nextPlan?.mailbox_limit ?? 1)
                        void applySubscription(subscription, { plan_code: event.target.value, purchased_mailbox_count: nextMailboxCount, reason: 'Platform admin changed plan' }, 'Business plan updated.')
                      }}
                    >
                      {plans.map((item) => <option key={item.code} value={item.code}>{item.name}</option>)}
                    </select>
                  </label>
                  <label>
                    Mailboxes
                    <input
                      type="number"
                      min={plan?.mailbox_limit ?? 1}
                      max={plan?.max_mailboxes ?? 500}
                      defaultValue={subscription.purchased_mailbox_count}
                      aria-label={`Purchased mailboxes for ${subscription.organization_name}`}
                      disabled={busy || subscription.is_system}
                      onBlur={(event) => {
                        const count = Number(event.target.value) || subscription.purchased_mailbox_count
                        if (count !== subscription.purchased_mailbox_count) void applySubscription(subscription, { purchased_mailbox_count: count, reason: 'Platform admin changed purchased mailbox quantity' }, 'Purchased mailbox quantity updated.')
                      }}
                    />
                  </label>
                  <label>
                    Status
                    <select
                      aria-label={`Subscription status for ${subscription.organization_name}`}
                      value={subscription.status}
                      disabled={busy || subscription.is_system}
                      onChange={(event) => void applySubscription(subscription, { status: event.target.value as BusinessSubscriptionRow['status'], reason: 'Platform admin changed subscription status' }, 'Subscription status updated.')}
                    >
                      <option value="trial">Trial</option>
                      <option value="active">Active</option>
                      <option value="past_due">Past due</option>
                      <option value="suspended">Suspended</option>
                      <option value="cancelled">Cancelled</option>
                    </select>
                  </label>
                  <label>
                    Expiration
                    <input
                      type="date"
                      value={expiryDate}
                      disabled={busy || subscription.is_system}
                      onChange={(event) => {
                        if (!event.target.value) return
                        void applySubscription(subscription, { current_period_end: new Date(`${event.target.value}T23:59:59.000Z`).toISOString(), reason: 'Platform admin changed subscription expiration' }, 'Subscription expiration updated.')
                      }}
                    />
                  </label>
                  <button
                    type="button"
                    className="secondary-button"
                    disabled={busy || subscription.is_system}
                    onClick={() => {
                      const base = subscription.current_period_end && new Date(subscription.current_period_end) > new Date() ? new Date(subscription.current_period_end) : new Date()
                      if (plan?.interval === 'month') base.setMonth(base.getMonth() + 1)
                      else base.setFullYear(base.getFullYear() + 1)
                      void applySubscription(subscription, { current_period_end: base.toISOString(), status: 'active', reason: `Platform admin extended ${plan?.interval === 'month' ? 'one month' : 'one year'}` }, 'Subscription extended and activated.')
                    }}
                  >
                    Extend {plan?.interval === 'month' ? '1 month' : '1 year'}
                  </button>
                </div>
                {openHistoryOrg === subscription.organization_id && (
                  <div className="admin-subscription-history">
                    <strong>Subscription history</strong>
                    {history.length === 0 && <small>No history entries yet.</small>}
                    {history.map((entry) => (
                      <div className="admin-history-row" key={entry.id}>
                        <span>{new Date(entry.assigned_at).toLocaleString()}</span>
                        <span>{entry.event_type}</span>
                        <span>{entry.plan_name} · {entry.purchased_mailbox_count} mailboxes</span>
                        <span>{entry.status_after ?? '—'}</span>
                        <span>{entry.reason || entry.assignment_source.replaceAll('_', ' ')}</span>
                        <span>{entry.invoice_number ?? entry.assigned_by_email ?? 'system'}</span>
                      </div>
                    ))}
                  </div>
                )}
              </div>
            )
          })}
          {!focusedBusinessId && subscriptionTotal > subscriptionPageSize && (
            <div className="admin-pagination">
              <button type="button" className="secondary-button" disabled={subscriptionPage === 0} onClick={() => setSubscriptionPage((page) => Math.max(0, page - 1))}>Previous</button>
              <span>Page {subscriptionPage + 1} of {Math.max(1, Math.ceil(subscriptionTotal / subscriptionPageSize))}</span>
              <button type="button" className="secondary-button" disabled={(subscriptionPage + 1) * subscriptionPageSize >= subscriptionTotal} onClick={() => setSubscriptionPage((page) => page + 1)}>Next</button>
            </div>
          )}
          {subscriptions.length === 0 && <p className="settings-hint">No business subscriptions match these filters.</p>}
        </section>
      )}

      {tab === 'plans' && (
        <>
          <section className="settings-section">
            <div className="admin-section-head">
              <h2>Plans</h2>
              <button type="button" className="secondary-button" onClick={startCreate}>
                <Plus size={14} />
                New plan
              </button>
            </div>
            {plans.map((plan) => (
              <div className="billing-row" key={plan.code}>
                <div>
                  <strong>
                    {plan.name} ({plan.code})
                  </strong>
                  <small>
                    {formatPrice(plan.price_cents, plan.currency)} base / {plan.interval} · {plan.mailbox_limit} included ·{' '}
                    {formatPrice(plan.extra_mailbox_price_cents, plan.currency)} per extra mailbox ·{' '}
                    {splitBytes(plan.mailbox_bytes).value} {splitBytes(plan.mailbox_bytes).unit} per mailbox ·{' '}
                    {plan.alias_limit_per_mailbox === null ? 'unlimited' : plan.alias_limit_per_mailbox} aliases/mailbox · max {plan.max_mailboxes} mailboxes
                  </small>
                </div>
                <div className="admin-actions">
                  <span className={plan.active ? 'billing-paid' : ''}>
                    {plan.active ? 'Active' : 'Hidden'}
                  </span>
                  <button
                    type="button"
                    className="secondary-button"
                    onClick={() => startEdit(plan)}
                  >
                    Edit
                  </button>
                  {plan.active && (
                    <button
                      type="button"
                      className="icon-button"
                      aria-label={`Deactivate ${plan.name}`}
                      disabled={busy}
                      onClick={() => void deactivatePlan(plan)}
                    >
                      <Trash2 size={15} />
                    </button>
                  )}
                </div>
              </div>
            ))}
          </section>

          {(editing || planForm.code || planForm.name || planForm.seats > 1) && (
            <form className="settings-section" onSubmit={savePlan}>
              <h2>{editing ? `Edit ${editing.name}` : 'Create a plan'}</h2>
              <div className="plan-form-grid">
                <label>
                  Code
                  <input
                    value={planForm.code}
                    disabled={Boolean(editing)}
                    onChange={(event) =>
                      setPlanForm((current) => ({ ...current, code: event.target.value }))
                    }
                    placeholder="starter"
                    aria-label="Plan code"
                  />
                </label>
                <label>
                  Name
                  <input
                    value={planForm.name}
                    onChange={(event) =>
                      setPlanForm((current) => ({ ...current, name: event.target.value }))
                    }
                    placeholder="CS Mail Starter"
                    aria-label="Plan name"
                  />
                </label>
                <label>
                  Price (cents)
                  <input
                    type="number"
                    min={0}
                    value={planForm.price_cents}
                    onChange={(event) =>
                      setPlanForm((current) => ({
                        ...current,
                        price_cents: Number(event.target.value) || 0,
                      }))
                    }
                    aria-label="Plan price in cents"
                  />
                </label>
                <label>
                  Extra mailbox price (cents)
                  <input
                    type="number"
                    min={0}
                    value={planForm.extra_mailbox_price_cents}
                    onChange={(event) => setPlanForm((current) => ({ ...current, extra_mailbox_price_cents: Number(event.target.value) || 0 }))}
                    aria-label="Additional mailbox price in cents"
                  />
                </label>
                <label>
                  Currency
                  <input
                    value={planForm.currency}
                    onChange={(event) =>
                      setPlanForm((current) => ({ ...current, currency: event.target.value }))
                    }
                    aria-label="Plan currency"
                  />
                </label>
                <label>
                  Interval
                  <select
                    value={planForm.interval}
                    onChange={(event) =>
                      setPlanForm((current) => ({ ...current, interval: event.target.value }))
                    }
                    aria-label="Billing interval"
                  >
                    <option value="month">Monthly</option>
                    <option value="year">Yearly</option>
                  </select>
                </label>
                <label>
                  Mailbox size (GB)
                  <input
                    type="number"
                    min={1}
                    value={Math.round(planForm.mailbox_bytes / (1024 * 1024 * 1024))}
                    onChange={(event) =>
                      setPlanForm((current) => ({
                        ...current,
                        mailbox_bytes: Math.round(
                          (Number(event.target.value) || 0) * 1024 * 1024 * 1024,
                        ),
                      }))
                    }
                    aria-label="Mailbox size in GB"
                  />
                </label>
                <label>
                  Max attachment (MB)
                  <input
                    type="number"
                    min={1}
                    max={100}
                    value={planForm.max_attachment_bytes / (1024 * 1024)}
                    onChange={(event) =>
                      setPlanForm((current) => ({
                        ...current,
                        max_attachment_bytes: Math.round(
                          (Number(event.target.value) || 0) * 1024 * 1024,
                        ),
                      }))
                    }
                    aria-label="Max attachment in MB"
                  />
                </label>
                <label>
                  Business storage pool (GB)
                  <input
                    type="number"
                    min={1}
                    value={Math.round(planForm.storage_pool_bytes / (1024 * 1024 * 1024))}
                    onChange={(event) =>
                      setPlanForm((current) => ({
                        ...current,
                        storage_pool_bytes: Math.round((Number(event.target.value) || 0) * 1024 * 1024 * 1024),
                      }))
                    }
                    aria-label="Business storage pool in GB"
                  />
                </label>
                <label>
                  Included mailboxes
                  <input type="number" min={1} value={planForm.mailbox_limit} onChange={(event) =>
                    setPlanForm((current) => ({ ...current, mailbox_limit: Number(event.target.value) || 1 }))
                  } aria-label="Included mailboxes" />
                </label>
                <label>
                  Maximum purchasable mailboxes
                  <input type="number" min={planForm.mailbox_limit} max={500} value={planForm.max_mailboxes} onChange={(event) =>
                    setPlanForm((current) => ({ ...current, max_mailboxes: Number(event.target.value) || current.mailbox_limit }))
                  } aria-label="Maximum purchasable mailboxes" />
                </label>
                <label>
                  Aliases per mailbox (blank = unlimited)
                  <input type="number" min={1} value={planForm.alias_limit_per_mailbox ?? ''} onChange={(event) =>
                    setPlanForm((current) => ({ ...current, alias_limit_per_mailbox: event.target.value ? Number(event.target.value) : null }))
                  } aria-label="Aliases per mailbox" />
                </label>
                <label>
                  Domain limit
                  <input type="number" min={1} value={planForm.domain_limit} onChange={(event) =>
                    setPlanForm((current) => ({ ...current, domain_limit: Number(event.target.value) || 1 }))
                  } aria-label="Domain limit" />
                </label>
                <label>
                  Business daily send limit (0 = unlimited)
                  <input type="number" min={0} value={planForm.organization_daily_send_limit} onChange={(event) =>
                    setPlanForm((current) => ({ ...current, organization_daily_send_limit: Number(event.target.value) || 0 }))
                  } aria-label="Business daily send limit" />
                </label>
                <label>
                  Recipients / message
                  <input
                    type="number"
                    min={1}
                    value={planForm.max_recipients}
                    onChange={(event) =>
                      setPlanForm((current) => ({
                        ...current,
                        max_recipients: Number(event.target.value) || 0,
                      }))
                    }
                    aria-label="Max recipients per message"
                  />
                </label>
                <label>
                  Daily send limit (0 = unlimited)
                  <input
                    type="number"
                    min={0}
                    value={planForm.daily_send_limit}
                    onChange={(event) =>
                      setPlanForm((current) => ({
                        ...current,
                        daily_send_limit: Number(event.target.value) || 0,
                      }))
                    }
                    aria-label="Daily send limit"
                  />
                </label>
                <label>
                  Seats
                  <input
                    type="number"
                    min={1}
                    value={planForm.seats}
                    onChange={(event) =>
                      setPlanForm((current) => ({
                        ...current,
                        seats: Number(event.target.value) || 1,
                      }))
                    }
                    aria-label="Seats"
                  />
                </label>
                <label>
                  Features (comma separated)
                  <input
                    value={(planForm.features ?? []).join(', ')}
                    onChange={(event) =>
                      setPlanForm((current) => ({
                        ...current,
                        features: event.target.value.split(','),
                      }))
                    }
                    placeholder="2 GB storage, 25 MB attachments"
                    aria-label="Plan features"
                  />
                </label>
              </div>
              <div className="settings-options">
                <label>
                  <input
                    type="checkbox"
                    checked={planForm.active}
                    onChange={(event) =>
                      setPlanForm((current) => ({ ...current, active: event.target.checked }))
                    }
                  />{' '}
                  Visible to customers
                </label>
                {entitlementFeatures.map(([key, label]) => (
                  <label key={key}>
                    <input
                      type="checkbox"
                      checked={planForm.feature_flags?.[key] ?? false}
                      onChange={(event) =>
                        setPlanForm((current) => ({
                          ...current,
                          feature_flags: {
                            ...(current.feature_flags ?? {}),
                            [key]: event.target.checked,
                          },
                        }))
                      }
                    />{' '}
                    {label}
                  </label>
                ))}
              </div>
              <div className="row-actions">
                <button
                  type="button"
                  className="secondary-button"
                  onClick={() => {
                    setEditing(null)
                    setPlanForm(emptyPlan())
                  }}
                >
                  Cancel
                </button>
                <button className="primary-button" disabled={busy}>
                  {busy ? 'Saving…' : 'Save plan'}
                </button>
              </div>
            </form>
          )}
        </>
      )}

      {tab === 'settings' && (
        <form className="settings-section" onSubmit={saveSettings}>
          <h2>Payment instructions</h2>
          <p className="settings-hint">
            Shown to customers when they order a plan. Keep these accurate — this is how they fund
            orders you verify below.
          </p>
          <div className="plan-form-grid">
            <label>
              Bank transfer details
              <textarea
                rows={3}
                value={settings.bank_details}
                onChange={(event) =>
                  setSettings((current) => ({ ...current, bank_details: event.target.value }))
                }
                placeholder="CS Mail · IBAN …"
                aria-label="Bank transfer details"
              />
            </label>
            <label>
              PayPal email
              <input
                value={settings.paypal_email}
                onChange={(event) =>
                  setSettings((current) => ({ ...current, paypal_email: event.target.value }))
                }
                placeholder="billing@crescentsphere.com"
                aria-label="PayPal email"
              />
            </label>
            <label>
              General instructions
              <textarea
                rows={3}
                value={settings.instructions}
                onChange={(event) =>
                  setSettings((current) => ({ ...current, instructions: event.target.value }))
                }
                placeholder="How customers should complete their payment…"
                aria-label="General payment instructions"
              />
            </label>
            <label>Seller legal name<input value={settings.seller_legal_name} onChange={(event) => setSettings((current) => ({ ...current, seller_legal_name: event.target.value }))} /></label>
            <label>Billing email<input type="email" value={settings.seller_email} onChange={(event) => setSettings((current) => ({ ...current, seller_email: event.target.value }))} /></label>
            <label>CR number<input value={settings.seller_cr_number} onChange={(event) => setSettings((current) => ({ ...current, seller_cr_number: event.target.value }))} /></label>
            <label>VAT number<input value={settings.seller_vat_number} onChange={(event) => setSettings((current) => ({ ...current, seller_vat_number: event.target.value }))} placeholder="Leave blank to issue invoices without VAT" /></label>
            <label>Seller address<textarea rows={2} value={settings.seller_address} onChange={(event) => setSettings((current) => ({ ...current, seller_address: event.target.value }))} /></label>
            <label>Tax rate (%)<input type="number" min="0" max="100" step="0.01" value={settings.tax_rate_bps / 100} onChange={(event) => setSettings((current) => ({ ...current, tax_rate_bps: Math.round(Number(event.target.value || 0) * 100) }))} /></label>
            <label>Invoice due days<input type="number" min="0" max="90" value={settings.invoice_due_days} onChange={(event) => setSettings((current) => ({ ...current, invoice_due_days: Number(event.target.value || 0) }))} /></label>
            <label>Grace days<input type="number" min="0" max="90" value={settings.grace_days} onChange={(event) => setSettings((current) => ({ ...current, grace_days: Number(event.target.value || 0) }))} /></label>
          </div>
          <div className="row-actions">
            <button className="primary-button" disabled={busy}>
              {busy ? 'Saving…' : 'Save settings'}
            </button>
          </div>
        </form>
      )}
    </div>
  )
}
