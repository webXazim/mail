import { useEffect, useState, type FormEvent } from 'react'
import { useNavigate } from 'react-router-dom'
import { ArrowLeft, Check, CreditCard, Landmark, Plus, Save, Trash2 } from 'lucide-react'
import {
  adminBillingApi,
  billingApi,
  formatPrice,
  friendlyError,
  paymentMethodLabel,
  splitBytes,
  type BillingSettingsRow,
  type OrderRow,
  type PlanView,
} from '../services/billing'

type AdminTab = 'orders' | 'plans' | 'settings'

const statusLabel: Record<OrderRow['status'], string> = {
  pending: 'Awaiting payment',
  submitted: 'Verifying payment',
  paid: 'Active',
  cancelled: 'Cancelled',
  rejected: 'Rejected',
}

const emptyPlan = (): PlanView => ({
  code: '',
  name: '',
  price_cents: 0,
  price: '$0.00',
  currency: 'USD',
  interval: 'month',
  mailbox_bytes: 25 * 1024 * 1024 * 1024,
  max_attachment_bytes: 25 * 1024 * 1024,
  max_recipients: 100,
  daily_send_limit: 500,
  seats: 5,
  features: [],
  active: true,
})

export function AdminBillingPage() {
  const navigate = useNavigate()
  const [tab, setTab] = useState<AdminTab>('orders')
  const [orders, setOrders] = useState<OrderRow[]>([])
  const [plans, setPlans] = useState<PlanView[]>([])
  const [settings, setSettings] = useState<BillingSettingsRow>({
    bank_details: '',
    paypal_email: '',
    instructions: '',
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

  const showNotice = (message: string) => {
    setNotice(message)
    window.setTimeout(() => setNotice(''), 5000)
  }

  const approveOrder = async (order: OrderRow) => {
    setBusy(true)
    try {
      await adminBillingApi.approveOrder(order.id, reviewNote.trim())
      setReviewId(null)
      setReviewNote('')
      showNotice(`${order.plan_name} activated for ${order.email} — invoice issued.`)
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
      currency: (planForm.currency || 'USD').toUpperCase(),
      interval: planForm.interval === 'year' ? 'year' : 'month',
      price_cents: Math.max(0, planForm.price_cents || 0),
      mailbox_bytes: Math.max(1, planForm.mailbox_bytes || 1),
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
                  {order.display_name || order.email} · {order.plan_name} ·{' '}
                  {formatPrice(order.amount_cents, order.currency)}
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
                  {order.admin_note ? ` · ${order.admin_note}` : ''}
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
                    {formatPrice(plan.price_cents, plan.currency)} / seat / {plan.interval} ·{' '}
                    {splitBytes(plan.mailbox_bytes).value} {splitBytes(plan.mailbox_bytes).unit}{' '}
                    mailbox · {plan.max_attachment_bytes / (1024 * 1024)} MB attachments ·{' '}
                    {plan.max_recipients} recipients · {plan.daily_send_limit || 'unlimited'}{' '}
                    sends/day · {plan.seats} seats
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
                    placeholder="Harbor Starter"
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
                placeholder="Harbor Mail Ltd · IBAN …"
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
                placeholder="billing@harbor.co"
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
