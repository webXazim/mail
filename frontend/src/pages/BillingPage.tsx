import { useEffect, useMemo, useState, type FormEvent } from 'react'
import { useNavigate } from 'react-router-dom'
import { ArrowLeft, Check, Download, FileText, Landmark, Star } from 'lucide-react'
import { downloadTextFile } from '../lib/files'
import { invoiceToText } from '../lib/invoices'
import { useProfile } from '../services/profile'
import {
  billingApi,
  friendlyError,
  formatPrice,
  invoiceFromOrder,
  paymentMethodLabel,
  splitBytes,
  summaryText,
  type BillingSummary,
  type OrderRow,
  type PlanView,
} from '../services/billing'

const statusLabel: Record<OrderRow['status'], string> = {
  pending: 'Awaiting payment',
  submitted: 'Verifying payment',
  paid: 'Active',
  cancelled: 'Cancelled',
  rejected: 'Rejected',
}

function instructionsFor(method: string, settings: BillingSummary['settings']): string {
  switch (method) {
    case 'bank':
      return settings.bank_details || 'Use the bank transfer details of Harbor Mail when paying.'
    case 'paypal':
      return settings.paypal_email
        ? `Pay via PayPal to ${settings.paypal_email} — put your order id in the note.`
        : 'Pay via PayPal to the address your admin provides.'
    case 'card':
      return 'Card payments are arranged after you place the order — our team shares a checkout link and confirms when it is received.'
    default:
      return (
        settings.instructions || 'Pay for your order using the instructions your admin provides.'
      )
  }
}

export function BillingPage() {
  const navigate = useNavigate()
  const profile = useProfile()
  const [summary, setSummary] = useState<BillingSummary | null>(null)
  const [plans, setPlans] = useState<PlanView[]>([])
  const [tab, setTab] = useState<'plan' | 'invoices'>('plan')
  const [ordering, setOrdering] = useState(false)
  const [chosenPlan, setChosenPlan] = useState('')
  const [method, setMethod] = useState('bank')
  const [note, setNote] = useState('')
  const [submittingId, setSubmittingId] = useState<string | null>(null)
  const [reference, setReference] = useState('')
  const [busy, setBusy] = useState(false)
  const [notice, setNotice] = useState('')
  const [downloaded, setDownloaded] = useState<string[]>([])

  const reload = async () => {
    const [next, active] = await Promise.all([billingApi.summary(), billingApi.plans()])
    setSummary(next)
    setPlans(active)
    setChosenPlan((current) => current || active[1]?.code || active[0]?.code || '')
    billingApi.refreshInvoices()
  }

  useEffect(() => {
    void billingApi.summary().then(setSummary)
    void billingApi.plans().then((active) => {
      setPlans(active)
      setChosenPlan((current) => current || active[1]?.code || active[0]?.code || '')
    })
    void billingApi.refreshInvoices()
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') navigate('/mail/inbox')
    }
    window.addEventListener('keydown', handleKeyDown)
    return () => window.removeEventListener('keydown', handleKeyDown)
  }, [navigate])

  const showNotice = (message: string) => {
    setNotice(message)
    window.setTimeout(() => setNotice(''), 5000)
  }

  const activePlans = plans.filter((plan) => plan.active)
  const pairedOrders = useMemo(() => {
    if (!summary) return []
    return summary.orders
      .filter((order) => order.status === 'paid')
      .sort((a, b) => (a.paid_at ?? a.created_at).localeCompare(b.paid_at ?? b.created_at))
      .reverse()
  }, [summary])

  if (!summary) {
    return (
      <div className="route-loader">
        <div className="loading-spinner" />
      </div>
    )
  }

  const current = summary.current_plan
  const storageTotal = summary.quota_bytes || current.mailbox_bytes
  const storageUsed = profile?.storage?.used_bytes ?? 0
  const storagePct = storageTotal > 0 ? Math.min(100, (storageUsed / storageTotal) * 100) : 0
  const { value: storageValue, unit: storageUnit } = splitBytes(storageTotal)

  const placeOrder = async (event: FormEvent) => {
    event.preventDefault()
    if (!chosenPlan || !method) return
    setBusy(true)
    try {
      await billingApi.createOrder(chosenPlan, method, note.trim())
      showNotice('Order placed — send the payment, then mark it paid below.')
      setOrdering(false)
      setNote('')
      await reload()
    } catch (error) {
      showNotice(friendlyError(error))
    } finally {
      setBusy(false)
    }
  }

  const submitPaid = async (order: OrderRow) => {
    if (!reference.trim()) return
    setBusy(true)
    try {
      await billingApi.submitPaid(order.id, order.payment_method, reference.trim())
      setSubmittingId(null)
      setReference('')
      showNotice('Reference submitted — we will verify and activate your plan.')
      await reload()
    } catch (error) {
      showNotice(friendlyError(error))
    } finally {
      setBusy(false)
    }
  }

  const cancelOrder = async (order: OrderRow) => {
    setBusy(true)
    try {
      await billingApi.cancelOrder(order.id)
      showNotice('Order cancelled.')
      await reload()
    } catch (error) {
      showNotice(friendlyError(error))
    } finally {
      setBusy(false)
    }
  }

  const downloadInvoice = (order: OrderRow) => {
    const invoice = invoiceFromOrder(order)
    if (!invoice) return
    downloadTextFile(`${invoice.id}.txt`, invoiceToText(invoice))
    setDownloaded((current) => [...current, invoice.id])
    window.setTimeout(
      () => setDownloaded((current) => current.filter((id) => id !== invoice.id)),
      3000,
    )
  }

  return (
    <div className="settings-page" role="region" aria-label="Billing">
      <header className="calendar-head">
        <div>
          <p className="eyebrow">Harbor Mail</p>
          <h1>Billing</h1>
        </div>
        <div className="calendar-head__actions">
          <button
            type="button"
            className="secondary-button"
            onClick={() => navigate('/mail/pricing')}
          >
            Compare plans
          </button>
          <button
            type="button"
            className="secondary-button"
            onClick={() => navigate('/mail/inbox')}
          >
            <ArrowLeft size={14} />
            Back to inbox
          </button>
        </div>
      </header>

      <nav className="admin-nav" aria-label="Billing sections">
        <button
          type="button"
          className={tab === 'plan' ? 'admin-nav--active' : ''}
          aria-current={tab === 'plan' ? 'page' : undefined}
          onClick={() => setTab('plan')}
        >
          <Star size={14} />
          Plan
        </button>
        <button
          type="button"
          className={tab === 'invoices' ? 'admin-nav--active' : ''}
          aria-current={tab === 'invoices' ? 'page' : undefined}
          onClick={() => setTab('invoices')}
        >
          <FileText size={14} />
          Invoices
        </button>
      </nav>

      {notice && <p className="settings-notice settings-notice--ok">{notice}</p>}

      {tab === 'plan' && (
        <>
          <section className="settings-section">
            <div className="admin-section-head">
              <h2>Current plan</h2>
              <span className="admin-section-count">
                {current.price} per seat / {current.interval}
              </span>
            </div>
            <div className="billing-plan">
              <div>
                <strong>{current.name}</strong>
                <small>
                  {current.features.join(' · ') || summaryText(current.daily_send_limit)}
                </small>
              </div>
              <button
                type="button"
                className="secondary-button"
                onClick={() => setOrdering((value) => !value)}
              >
                {ordering ? 'Close' : 'Change plan'}
              </button>
            </div>
            <div className="storage">
              <span>Storage used</span>
              <strong>
                {profile?.storage
                  ? `${profile.storage.pct.toFixed(0)}%`
                  : `${storageValue} ${storageUnit}`}
              </strong>
              <div className="storage-bar">
                <span style={{ width: `${storagePct}%` }} />
              </div>
            </div>
          </section>

          {ordering && (
            <form className="settings-section" onSubmit={placeOrder}>
              <h2>Order a plan</h2>
              <p className="settings-hint">
                Choose a plan and a payment method. You fund the order yourself (bank transfer,
                PayPal or card) and an admin verifies the payment to activate the plan.
              </p>
              {activePlans.map((plan) => (
                <button
                  type="button"
                  className={`plan-option ${chosenPlan === plan.code ? 'plan-option--active' : ''}`}
                  key={plan.code}
                  onClick={() => setChosenPlan(plan.code)}
                  aria-pressed={chosenPlan === plan.code}
                >
                  <span>
                    <strong>{plan.name}</strong>
                    <small>
                      {splitBytes(plan.mailbox_bytes).value} {splitBytes(plan.mailbox_bytes).unit}{' '}
                      per mailbox · {plan.max_recipients} recipients ·{' '}
                      {summaryText(plan.daily_send_limit)}
                    </small>
                  </span>
                  <span className="plan-option__price">
                    <strong>{plan.price}</strong>
                    <small>per seat / {plan.interval}</small>
                  </span>
                  {chosenPlan === plan.code && <Check size={15} />}
                </button>
              ))}
              <label>
                Payment method
                <select value={method} onChange={(event) => setMethod(event.target.value)}>
                  <option value="bank">Bank transfer</option>
                  <option value="paypal">PayPal</option>
                  <option value="card">Card</option>
                  <option value="other">Other</option>
                </select>
              </label>
              <div className="billing-pay-instructions">
                <strong>
                  {method === 'bank' ? 'Bank transfer' : paymentMethodLabel(method)} — how to pay
                </strong>
                <p>{instructionsFor(method, summary.settings)}</p>
              </div>
              <label>
                Order note (optional)
                <input
                  value={note}
                  onChange={(event) => setNote(event.target.value)}
                  maxLength={500}
                  placeholder="Anything we should know about this order"
                  aria-label="Order note"
                />
              </label>
              <div className="row-actions">
                <button
                  type="button"
                  className="secondary-button"
                  onClick={() => setOrdering(false)}
                >
                  Cancel
                </button>
                <button className="primary-button" disabled={busy || !chosenPlan}>
                  {busy ? 'Placing…' : 'Place order'}
                </button>
              </div>
            </form>
          )}

          <section className="settings-section">
            <div className="admin-section-head">
              <h2>Orders</h2>
              <span className="admin-section-count">{summary.orders.length} total</span>
            </div>
            {summary.orders.length === 0 && (
              <p className="settings-hint">
                No orders yet. Change plan to order a paid subscription, or keep the current plan as
                is.
              </p>
            )}
            {summary.orders.map((order) => (
              <div className="billing-row" key={order.id}>
                <div>
                  <strong>
                    {order.plan_name} · {formatPrice(order.amount_cents, order.currency)}
                  </strong>
                  <small>
                    {paymentMethodLabel(order.payment_method)} ·{' '}
                    {new Date(order.created_at).toLocaleString([], {
                      month: 'short',
                      day: 'numeric',
                      year: 'numeric',
                    })}
                    {order.invoice_number ? ` · ${order.invoice_number}` : ''}
                    {order.admin_note ? ` — ${order.admin_note}` : ''}
                  </small>
                </div>
                <div className="admin-actions">
                  {(order.status === 'paid' ||
                    order.status === 'cancelled' ||
                    order.status === 'rejected') && (
                    <span className={order.status === 'paid' ? 'billing-paid' : ''}>
                      {order.status === 'paid' && <Check size={13} />}
                      {statusLabel[order.status]}
                    </span>
                  )}
                  {order.status === 'submitted' && <span>Verifying payment</span>}
                  {order.status === 'pending' && (
                    <>
                      {submittingId === order.id ? (
                        <div className="admin-inline-form">
                          <input
                            value={reference}
                            onChange={(event) => setReference(event.target.value)}
                            placeholder="Bank / PayPal reference"
                            aria-label="Payment reference"
                          />
                          <button
                            type="button"
                            className="primary-button"
                            disabled={busy || !reference.trim()}
                            onClick={() => void submitPaid(order)}
                          >
                            Submit
                          </button>
                        </div>
                      ) : (
                        <button
                          type="button"
                          className="secondary-button"
                          onClick={() => {
                            setSubmittingId(order.id)
                            setReference('')
                          }}
                        >
                          <Landmark size={13} />
                          I&apos;ve paid
                        </button>
                      )}
                      <button
                        type="button"
                        className="icon-button"
                        aria-label={`Cancel order ${order.plan_name}`}
                        disabled={busy}
                        onClick={() => void cancelOrder(order)}
                      >
                        <FileText size={15} className="admin-record--pending" />
                      </button>
                    </>
                  )}
                </div>
              </div>
            ))}
          </section>
        </>
      )}

      {tab === 'invoices' && (
        <section className="settings-section">
          <div className="admin-section-head">
            <h2>Invoices</h2>
            <span className="admin-section-count">{pairedOrders.length} invoices</span>
          </div>
          {pairedOrders.length === 0 && (
            <p className="settings-hint">
              Paid orders appear here as invoices once an admin confirms the payment.
            </p>
          )}
          {pairedOrders.map((order) => {
            const invoiceId = order.invoice_number ?? order.id
            return (
              <div className="billing-row" key={order.id}>
                <div>
                  <strong>{invoiceId}</strong>
                  <small>
                    {order.paid_at
                      ? new Date(order.paid_at).toLocaleDateString([], {
                          month: 'long',
                          day: 'numeric',
                          year: 'numeric',
                        })
                      : ''}
                    {' · '}
                    {formatPrice(order.amount_cents, order.currency)} · {order.plan_name} ·{' '}
                    {paymentMethodLabel(order.payment_method)}
                  </small>
                </div>
                <span className="billing-paid">
                  <Check size={13} />
                  Paid
                </span>
                <div className="admin-actions">
                  {downloaded.includes(invoiceId) ? (
                    <small className="settings-notice settings-notice--ok">Downloaded</small>
                  ) : (
                    <button
                      type="button"
                      className="secondary-button"
                      onClick={() => downloadInvoice(order)}
                    >
                      <Download size={13} />
                      Download
                    </button>
                  )}
                  <button
                    type="button"
                    className="secondary-button"
                    onClick={() => navigate(`/mail/billing/invoices/${invoiceId}`)}
                  >
                    <FileText size={13} />
                    View receipt
                  </button>
                </div>
              </div>
            )
          })}
        </section>
      )}
    </div>
  )
}
