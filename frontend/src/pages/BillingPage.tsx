import { useEffect, useMemo, useState, type FormEvent } from 'react'
import { useNavigate, useSearchParams } from 'react-router-dom'
import { ArrowLeft, Check, FileText, Landmark, Save, Star } from 'lucide-react'
import {
  billingApi,
  friendlyError,
  formatPrice,
  paymentMethodLabel,
  splitBytes,
  summaryText,
  type BillingProfileRow,
  type BillingSummary,
  type OrderRow,
  type PlanView,
} from '../services/billing'
import { profileApi, useProfile } from '../services/profile'
import { PlanOnboardingPage } from './PlanOnboardingPage'

const statusLabel: Record<OrderRow['status'], string> = {
  pending: 'Payment due',
  submitted: 'Payment under review',
  paid: 'Paid',
  cancelled: 'Cancelled',
  rejected: 'Rejected',
}

function instructionsFor(method: string, settings: BillingSummary['settings']): string {
  switch (method) {
    case 'bank':
      return settings.bank_details || 'Use the bank transfer details of CS Mail when paying.'
    case 'paypal':
      return settings.paypal_email
        ? `Pay via PayPal to ${settings.paypal_email} — put your order id in the note.`
        : 'Pay via PayPal to the address your admin provides.'
    default:
      return (
        settings.instructions || 'Pay for your order using the instructions your admin provides.'
      )
  }
}

const poolFor = (plan: PlanView, count: number) =>
  plan.storage_pool_bytes + Math.max(0, count - plan.mailbox_limit) * plan.mailbox_bytes

const formatBillingDate = (value: string | null) => value
  ? new Date(value).toLocaleDateString([], { month: 'short', day: 'numeric', year: 'numeric' })
  : '—'

const isReduction = (plan: PlanView, current: PlanView, currentCount: number, targetCount = Math.max(currentCount, plan.mailbox_limit)) => {
  const count = Math.max(plan.mailbox_limit, targetCount)
  return count < currentCount || count > plan.max_mailboxes || plan.price_cents < current.price_cents || plan.mailbox_bytes < current.mailbox_bytes
    || poolFor(plan, count) < poolFor(current, currentCount)
    || plan.domain_limit < current.domain_limit || plan.max_attachment_bytes < current.max_attachment_bytes
    || plan.max_recipients < current.max_recipients || plan.organization_daily_send_limit < current.organization_daily_send_limit
    || (current.alias_limit_per_mailbox == null ? plan.alias_limit_per_mailbox != null
      : plan.alias_limit_per_mailbox != null && plan.alias_limit_per_mailbox < current.alias_limit_per_mailbox)
}

export function BillingPage() {
  const profile = useProfile()
  if (!profile?.active_organization?.id) return <PlanOnboardingPage />
  return <BusinessBillingPage />
}

function BusinessBillingPage() {
  const navigate = useNavigate()
  const [searchParams] = useSearchParams()
  const [summary, setSummary] = useState<BillingSummary | null>(null)
  const [plans, setPlans] = useState<PlanView[]>([])
  const [tab, setTab] = useState<'plan' | 'invoices'>('plan')
  const [ordering, setOrdering] = useState(false)
  const [chosenPlan, setChosenPlan] = useState(searchParams.get('plan') || '')
  const [mailboxCount, setMailboxCount] = useState(1)
  const [method, setMethod] = useState('bank')
  const [note, setNote] = useState('')
  const [submittingId, setSubmittingId] = useState<string | null>(null)
  const [reference, setReference] = useState('')
  const [busy, setBusy] = useState(false)
  const [notice, setNotice] = useState('')
  const [loadError, setLoadError] = useState('')
  const [billingProfile, setBillingProfile] = useState<BillingProfileRow | null>(null)

  const reload = async () => {
    setLoadError('')
    const [next, active] = await Promise.all([billingApi.summary(), billingApi.plans()])
    setSummary(next)
    setBillingProfile(next.billing_profile)
    setPlans(active)
    const purgeInProgress = Boolean(next.purge_started_at && !next.data_purged_at)
    if (next.subscription_status !== 'active' && next.subscription_status !== 'trial'
      && !purgeInProgress
      && !next.orders.some((order) => order.invoice_status === 'issued' && (order.status === 'pending' || order.status === 'submitted'))) setOrdering(true)
    const defaultPlan = active[1] ?? active[0]
    const hasCurrentPlan = ['active', 'trial', 'past_due'].includes(next.subscription_status)
      || (next.subscription_status === 'suspended' && next.assignment_source !== 'bootstrap')
    const scheduledRenewal = next.scheduled_change?.status === 'ready_for_renewal'
      ? active.find((plan) => plan.code === next.scheduled_change?.target_plan_code)
      : undefined
    const initialPlan = scheduledRenewal ?? (hasCurrentPlan ? next.current_plan : defaultPlan)
    setChosenPlan((selected) => scheduledRenewal?.code
      ?? (active.some((plan) => plan.code === selected) ? selected : (hasCurrentPlan ? next.current_plan.code : defaultPlan?.code || '')))
    if (scheduledRenewal && next.scheduled_change?.target_mailbox_count) {
      setMailboxCount(next.scheduled_change.target_mailbox_count)
    } else {
      setMailboxCount((current) => Math.max(current, next.mailbox_limit, next.usage.mailboxes, next.usage.seats, initialPlan?.mailbox_limit ?? 1))
    }
    billingApi.refreshInvoices()
    await profileApi.refresh().catch(() => undefined)
  }

  useEffect(() => {
    const initialLoad = window.setTimeout(() => {
      void reload().catch((error) => setLoadError(friendlyError(error)))
      void billingApi.refreshInvoices()
    }, 0)
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') navigate('/mail/inbox')
    }
    window.addEventListener('keydown', handleKeyDown)
    return () => {
      window.clearTimeout(initialLoad)
      window.removeEventListener('keydown', handleKeyDown)
    }
    // reload is intentionally mount-only; user-triggered retries call it directly.
  }, [navigate])

  const showNotice = (message: string) => {
    setNotice(message)
    window.setTimeout(() => setNotice(''), 5000)
  }

  const activePlans = plans.filter((plan) => plan.active)
  const pairedOrders = useMemo(() => {
    if (!summary) return []
    return summary.orders
      .filter((order) => Boolean(order.invoice_number))
      .sort((a, b) => (a.issued_at ?? a.created_at).localeCompare(b.issued_at ?? b.created_at))
      .reverse()
  }, [summary])

  if (loadError) {
    return <div className="settings-page" role="alert"><h1>Plans and billing could not load</h1><p>{loadError}</p><button className="primary-button" onClick={() => void reload().catch((error) => setLoadError(friendlyError(error)))}>Try again</button></div>
  }
  if (!summary) {
    return (
      <div className="route-loader">
        <div className="loading-spinner" />
      </div>
    )
  }

  const current = summary.current_plan
  const planActive = summary.subscription_status === 'active' || summary.subscription_status === 'trial'
  const hasCurrentPlan = planActive || summary.subscription_status === 'past_due'
    || (summary.subscription_status === 'suspended' && summary.assignment_source !== 'bootstrap')
  const mailAccessAvailable = planActive || summary.subscription_status === 'past_due'
  const purgeInProgress = Boolean(summary.purge_started_at && !summary.data_purged_at)
  const openInvoice = summary.orders.find((order) => order.invoice_status === 'issued' && (order.status === 'pending' || order.status === 'submitted'))
  const storageTotal = summary.storage_pool_bytes || current.storage_pool_bytes || summary.quota_bytes || current.mailbox_bytes
  const storageAllocated = summary.storage_allocated_bytes ?? 0
  const storageUsed = summary.storage_used_bytes ?? 0
  const storagePct = storageTotal > 0 ? Math.min(100, (storageAllocated / storageTotal) * 100) : 0
  const { value: storageValue, unit: storageUnit } = splitBytes(storageTotal)
  const selectedPlan = activePlans.find((plan) => plan.code === chosenPlan) ?? activePlans[0]
  const selectedReduction = Boolean(planActive && selectedPlan && isReduction(selectedPlan, current, summary.mailbox_limit, mailboxCount))
  const selectedExtraCount = selectedPlan ? Math.max(0, mailboxCount - selectedPlan.mailbox_limit) : 0
  const selectedSubtotal = selectedPlan ? selectedPlan.price_cents + selectedExtraCount * selectedPlan.extra_mailbox_price_cents : 0
  const selectedTaxBps = summary.settings.seller_vat_number ? summary.settings.tax_rate_bps : 0
  const selectedTax = Math.round(selectedSubtotal * selectedTaxBps / 10000)

  const placeOrder = async (event: FormEvent) => {
    event.preventDefault()
    if (purgeInProgress) {
      showNotice('Retained-data purge is in progress. Contact support before placing another plan order.')
      return
    }
    if (!chosenPlan || !method) return
    setBusy(true)
    try {
      if (selectedReduction) {
        const change = await billingApi.schedulePlanChange(chosenPlan, mailboxCount, note.trim())
        showNotice(`${change.target_plan_name ?? chosenPlan} scheduled for ${formatBillingDate(change.effective_at)}. Your current paid plan remains unchanged until then.`)
      } else {
        const order = await billingApi.createOrder(chosenPlan, mailboxCount, method, note.trim())
        showNotice(order.activation_mode === 'test_instant'
          ? `${order.plan_name} activated immediately for testing. Invoice ${order.invoice_number ?? ''} was issued and payment is still due.`
          : `Invoice ${order.invoice_number ?? ''} issued. Pay it and submit the reference for activation.`)
      }
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
      showNotice(summary.instant_activation ? 'Payment reference submitted for review. Your test plan remains active.' : 'Payment reference submitted — activation follows verification.')
      await reload()
    } catch (error) {
      showNotice(friendlyError(error))
    } finally {
      setBusy(false)
    }
  }

  const cancelOrder = async (order: OrderRow) => {
    if (!window.confirm(`Cancel invoice ${order.invoice_number ?? ''}?${order.activation_mode === 'test_instant' ? ' Access granted by this test order will be suspended.' : ''}`)) return
    setBusy(true)
    try {
      await billingApi.cancelOrder(order.id)
      showNotice('Invoice cancelled. You can now place a new order.')
      setOrdering(false)
      await reload()
    } catch (error) {
      showNotice(friendlyError(error))
    } finally {
      setBusy(false)
    }
  }

  const scheduleCancellation = async () => {
    if (!summary.current_period_end || !window.confirm(`End this subscription at the close of the current paid term on ${formatBillingDate(summary.current_period_end)}? Mail access will continue until then.`)) return
    setBusy(true)
    try {
      await billingApi.scheduleCancellation()
      showNotice(`Cancellation scheduled for ${formatBillingDate(summary.current_period_end)}. You can undo it before the term ends.`)
      await reload()
    } catch (error) { showNotice(friendlyError(error)) } finally { setBusy(false) }
  }

  const cancelScheduledChange = async () => {
    setBusy(true)
    try {
      await billingApi.cancelScheduledChange()
      showNotice('Scheduled subscription change cancelled. Your current plan will continue unless you make another change.')
      await reload()
    } catch (error) { showNotice(friendlyError(error)) } finally { setBusy(false) }
  }

  const saveBillingProfile = async () => {
    if (!billingProfile) return
    setBusy(true)
    try {
      const next = await billingApi.updateProfile(billingProfile)
      setBillingProfile(next)
      setSummary((current) => current ? { ...current, billing_profile: next } : current)
      showNotice('Billing details saved. New invoices will snapshot these details.')
    } catch (error) {
      showNotice(friendlyError(error))
    } finally {
      setBusy(false)
    }
  }


  return (
    <div className="settings-page" role="region" aria-label="Billing">
      <header className="calendar-head">
        <div>
          <p className="eyebrow">CS Mail</p>
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

      {notice && <p className="settings-notice" role="status">{notice}</p>}

      {tab === 'plan' && (
        <>
          {summary.subscription_status === 'past_due' && <section className="settings-section" role="status">
            <h2>Renewal overdue — grace period active</h2>
            <p>Your existing mail access remains available{summary.renewal_grace_end ? ` until ${formatBillingDate(summary.renewal_grace_end)}` : ' during the renewal grace period'}. New mailboxes, domains, storage increases and app passwords are paused until renewal is approved.</p>
          </section>}
          {summary.subscription_status === 'suspended' && hasCurrentPlan && <section className="settings-section" role="alert">
            <h2>Subscription suspended</h2>
            <p>Mail access is disabled, but your business and mailbox data are retained for recovery{summary.data_retention_until ? ` until ${formatBillingDate(summary.data_retention_until)}` : ''}. Renew an eligible plan and complete payment approval to restore provider access.</p>
          </section>}
          {summary.retention_expired_at && !summary.purge_started_at && !summary.data_purged_at && <section className="settings-section" role="alert">
            <h2>Data-retention deadline reached</h2>
            <p>Your retained mailbox data has not been deleted automatically. It is now eligible for an explicit platform-admin purge. Renew and complete payment approval before a purge begins to restore the retained mailbox data.</p>
          </section>}
          {purgeInProgress && <section className="settings-section" role="alert">
            <h2>Retained-data purge in progress</h2>
            <p>Mailbox/provider deletion has already started, so new plan orders and reactivation are temporarily blocked. Contact support if you need help with this account.</p>
          </section>}
          {summary.data_purged_at && <section className="settings-section" role="status">
            <h2>Previous retained mailbox data was purged</h2>
            <p>The earlier retained mailbox data was permanently removed on {formatBillingDate(summary.data_purged_at)}. You can purchase a new term, but it starts as a fresh mailbox service and cannot restore the purged messages.</p>
          </section>}
          {summary.scheduled_change && <section className="settings-section" role="status">
            <div className="admin-section-head">
              <div>
                <h2>{summary.scheduled_change.change_type === 'cancel' ? 'Cancellation scheduled' : 'Plan change scheduled'}</h2>
                <p className="settings-hint">
                  {summary.scheduled_change.change_type === 'cancel'
                    ? `Your current plan stays active through ${formatBillingDate(summary.scheduled_change.effective_at)}. Mail access ends after the paid term unless you cancel this request or renew.`
                    : `${summary.scheduled_change.target_plan_name ?? summary.scheduled_change.target_plan_code} with ${summary.scheduled_change.target_mailbox_count ?? 'the selected'} mailboxes is scheduled for renewal on ${formatBillingDate(summary.scheduled_change.effective_at)}.`}
                </p>
                {summary.scheduled_change.blocked_reason && <p className="settings-hint">Before renewal: {summary.scheduled_change.blocked_reason}.</p>}
                {summary.scheduled_change.status === 'ready_for_renewal' && summary.scheduled_change.change_type === 'plan_change' && <p className="settings-hint">The current term has ended. Create and pay the renewal invoice for the scheduled plan to apply it.</p>}
              </div>
              <button type="button" className="secondary-button" disabled={busy} onClick={() => void cancelScheduledChange()}>Cancel scheduled change</button>
            </div>
          </section>}
          {openInvoice && <section className="settings-section" role="status">
            <h2>Complete or cancel your open invoice</h2>
            <p>Invoice {openInvoice.invoice_number} for {openInvoice.plan_name} is {openInvoice.status === 'submitted' ? 'awaiting payment review' : 'unpaid'}. A second order cannot be placed while it is open.</p>
            {planActive && openInvoice.activation_mode === 'payment_approval' && <p>Your current plan remains active. The ordered plan will replace it only after payment approval.</p>}
            {!planActive && summary.instant_activation && openInvoice.activation_mode === 'payment_approval' && <p>This invoice awaits payment approval and will not activate retroactively. {openInvoice.status === 'pending' ? 'You may cancel the unpaid invoice and place a new order.' : 'Its submitted payment must be reviewed before another order can be placed.'}</p>}
            {openInvoice.status === 'pending' && <button type="button" className="secondary-button" disabled={busy} onClick={() => void cancelOrder(openInvoice)}>Cancel invoice {openInvoice.invoice_number}</button>}
            {openInvoice.status === 'submitted' && <p>Payment was submitted for review. Contact billing support before replacing this invoice.</p>}
          </section>}
          <section className="settings-section">
            {planActive && <div className="row-actions"><button type="button" className="primary-button" onClick={() => navigate('/mail/business')}>Continue to domain setup</button></div>}
            {!mailAccessAvailable && openInvoice && !summary.instant_activation && <p className="settings-hint" role="status">Your invoice is awaiting payment. Mail access and capacity management will be available when the order is approved.</p>}
            <div className="admin-section-head">
              <h2>{hasCurrentPlan ? 'Current plan' : 'Choose and activate a plan'}</h2>
              <span className="admin-section-count">{hasCurrentPlan ? `${current.price} base / ${current.interval} · ${summary.mailbox_limit} mailboxes purchased` : 'Activation pending'}</span>
            </div>
            <div className="billing-plan">
              <div>
                <strong>{hasCurrentPlan ? current.name : 'No active plan'}</strong>
                <small>
                  {hasCurrentPlan ? current.features.join(' · ') || summaryText(current.daily_send_limit) : openInvoice ? 'Resolve the open invoice above before setting up your domain.' : summary.instant_activation ? 'Place a test order to activate a plan before setting up your domain.' : 'Choose a plan and complete payment approval before setting up your domain.'}
                </small>
                {hasCurrentPlan && <small>Current term: {formatBillingDate(summary.current_period_start)} → {formatBillingDate(summary.current_period_end)}</small>}
              </div>
              <button
                type="button"
                className="secondary-button"
                disabled={Boolean(openInvoice) || purgeInProgress}
                onClick={() => setOrdering((value) => !value)}
              >
              {ordering ? 'Close' : planActive ? 'Upgrade or renew' : hasCurrentPlan ? 'Renew or change plan' : 'Choose plan'}
              </button>
              {planActive && !summary.scheduled_change && <button type="button" className="text-button" disabled={busy} onClick={() => void scheduleCancellation()}>Cancel at renewal</button>}
            </div>
            {hasCurrentPlan && <div className="storage">
              <span>Storage pool</span>
              <strong>{storagePct.toFixed(0)}% of {storageValue} {storageUnit}</strong>
              <div className="storage-bar">
                <span style={{ width: `${storagePct}%` }} />
              </div>
              <small>{splitBytes(storageUsed).value} {splitBytes(storageUsed).unit} used · {splitBytes(storageAllocated).value} {splitBytes(storageAllocated).unit} reserved · {splitBytes(Math.max(0, storageTotal - storageAllocated)).value} {splitBytes(Math.max(0, storageTotal - storageAllocated)).unit} unreserved.</small>
              <button type="button" className="text-button" onClick={() => navigate('/mail/business')}>Manage mailbox storage</button>
            </div>}
          </section>

          <section className="settings-section">
            <div className="admin-section-head">
              <div>
                <h2>Billing details</h2>
                <p className="settings-hint">These details are snapshotted onto new invoices.</p>
              </div>
              <button type="button" className="secondary-button" disabled={busy || !billingProfile} onClick={() => void saveBillingProfile()}>
                <Save size={14} /> Save details
              </button>
            </div>
            {billingProfile && (
              <div className="settings-grid">
                <label>Legal / business name<input value={billingProfile.legal_name} onChange={(event) => setBillingProfile({ ...billingProfile, legal_name: event.target.value })} /></label>
                <label>Billing email<input type="email" value={billingProfile.billing_email} onChange={(event) => setBillingProfile({ ...billingProfile, billing_email: event.target.value })} /></label>
                <label>VAT number<input value={billingProfile.vat_number} onChange={(event) => setBillingProfile({ ...billingProfile, vat_number: event.target.value })} /></label>
                <label>CR number<input value={billingProfile.cr_number} onChange={(event) => setBillingProfile({ ...billingProfile, cr_number: event.target.value })} /></label>
                <label>Address<input value={billingProfile.address_line1} onChange={(event) => setBillingProfile({ ...billingProfile, address_line1: event.target.value })} /></label>
                <label>City<input value={billingProfile.city} onChange={(event) => setBillingProfile({ ...billingProfile, city: event.target.value })} /></label>
                <label>Postal code<input value={billingProfile.postal_code} onChange={(event) => setBillingProfile({ ...billingProfile, postal_code: event.target.value })} /></label>
                <label>Country<input value={billingProfile.country} onChange={(event) => setBillingProfile({ ...billingProfile, country: event.target.value })} /></label>
              </div>
            )}
          </section>

          {ordering && !openInvoice && !purgeInProgress && (
            <form className="settings-section" onSubmit={placeOrder}>
              <h2>{planActive ? 'Upgrade or renew your plan' : hasCurrentPlan ? 'Renew or change your plan' : 'Order a plan'}</h2>
              <p className="settings-hint">
                {planActive
                  ? 'Your current plan remains active while a new invoice is reviewed. A same-plan renewal with the same mailbox quantity extends from your existing expiry; an approved upgrade starts a new billing term. Reductions can be scheduled for the renewal date without shortening your current paid access.'
                  : hasCurrentPlan
                  ? 'Choose the plan you want to reactivate. Existing mail access is restored only after payment approval; capacity limits are validated before the invoice can be applied.'
                  : summary.instant_activation
                  ? 'Testing mode is enabled: placing the order activates the plan immediately and issues an invoice. Payment remains due and can still be submitted for manual review.'
                  : 'Choose a plan and payment method. The invoice is issued immediately; the plan activates after manual payment verification.'}
              </p>
              {activePlans.map((plan) => (
                <button
                  type="button"
                  className={`plan-option ${chosenPlan === plan.code ? 'plan-option--active' : ''}`}
                  key={plan.code}
                  disabled={false}
                  onClick={() => {
                    setChosenPlan(plan.code)
                    setMailboxCount(Math.max(plan.mailbox_limit, summary.usage.mailboxes, summary.usage.seats))
                  }}
                  aria-pressed={chosenPlan === plan.code}
                >
                  <span>
                    <strong>{plan.name}</strong>
                    <small>
                      {plan.mailbox_limit} included · {splitBytes(plan.mailbox_bytes).value} {splitBytes(plan.mailbox_bytes).unit}{' '}
                      per mailbox · extra mailbox {formatPrice(plan.extra_mailbox_price_cents, plan.currency)} / {plan.interval}
                    </small>
                  </span>
                  <span className="plan-option__price">
                    <strong>{plan.price}</strong>
                    <small>base / {plan.interval}</small>
                  </span>
                  {chosenPlan === plan.code && <Check size={15} />}
                  {planActive && isReduction(plan, current, summary.mailbox_limit) && <small>Schedule for renewal — current paid term stays unchanged</small>}
                </button>
              ))}
              {selectedPlan && (
                <>
                  <label>
                    Mailboxes
                    <input
                      type="number"
                      min={selectedReduction ? selectedPlan.mailbox_limit : Math.max(selectedPlan.mailbox_limit, summary.usage.mailboxes, summary.usage.seats)}
                      max={selectedPlan.max_mailboxes}
                      value={mailboxCount}
                      onChange={(event) => {
                        const minimum = selectedReduction ? selectedPlan.mailbox_limit : Math.max(selectedPlan.mailbox_limit, summary.usage.mailboxes, summary.usage.seats)
                        const next = Number(event.target.value) || minimum
                        setMailboxCount(Math.max(minimum, Math.min(selectedPlan.max_mailboxes, next)))
                      }}
                      aria-label="Mailbox quantity"
                    />
                    <small>{selectedPlan.mailbox_limit} included; up to {selectedPlan.max_mailboxes}. Each additional mailbox is {formatPrice(selectedPlan.extra_mailbox_price_cents, selectedPlan.currency)} / {selectedPlan.interval}.</small>
                  </label>
                  <div className="billing-pay-instructions">
                    <strong>Order total before payment</strong>
                    <p>Base {formatPrice(selectedPlan.price_cents, selectedPlan.currency)} + {selectedExtraCount} additional mailbox{selectedExtraCount === 1 ? '' : 'es'} = {formatPrice(selectedSubtotal, selectedPlan.currency)} subtotal{selectedTaxBps > 0 ? ` + ${formatPrice(selectedTax, selectedPlan.currency)} VAT` : ''}. Total {formatPrice(selectedSubtotal + selectedTax, selectedPlan.currency)}.</p>
                  </div>
                </>
              )}
              {selectedReduction ? (
                <div className="billing-pay-instructions">
                  <strong>No payment is collected now</strong>
                  <p>This schedules the lower plan or mailbox quantity for the current term end. Your current paid capacity stays unchanged. When the renewal date arrives, Billing will ask you to place and pay the renewal invoice before the scheduled reduction is activated.</p>
                </div>
              ) : (
                <>
                  <label>
                    Payment method
                    <select value={method} onChange={(event) => setMethod(event.target.value)}>
                      <option value="bank">Bank transfer</option>
                      {summary.settings.paypal_email && <option value="paypal">PayPal</option>}
                      <option value="other">Other manual payment</option>
                    </select>
                  </label>
                  <div className="billing-pay-instructions">
                    <strong>
                      {method === 'bank' ? 'Bank transfer' : paymentMethodLabel(method)} — how to pay
                    </strong>
                    <p>{instructionsFor(method, summary.settings)}</p>
                  </div>
                </>
              )}
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
                <button className="primary-button" disabled={busy || !chosenPlan || purgeInProgress}>
                  {busy ? (selectedReduction ? 'Scheduling…' : 'Placing…') : selectedReduction ? 'Schedule for renewal' : 'Place order'}
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
                    {order.plan_name} · {formatPrice(order.total_cents, order.currency)}
                  </strong>
                  <small>
                    {paymentMethodLabel(order.payment_method)} ·{' '}
                    {new Date(order.created_at).toLocaleString([], {
                      month: 'short',
                      day: 'numeric',
                      year: 'numeric',
                    })}
                    {` · ${order.mailbox_count} mailbox${order.mailbox_count === 1 ? '' : 'es'}`}
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
              Issued invoices appear here immediately when a plan is ordered.
            </p>
          )}
          {pairedOrders.map((order) => {
            const invoiceId = order.invoice_number ?? order.id
            return (
              <div className="billing-row" key={order.id}>
                <div>
                  <strong>{invoiceId}</strong>
                  <small>
                    {order.issued_at
                      ? new Date(order.issued_at).toLocaleDateString([], {
                          month: 'long',
                          day: 'numeric',
                          year: 'numeric',
                        })
                      : ''}
                    {' · '}
                    {formatPrice(order.total_cents, order.currency)} · {order.plan_name} · {order.mailbox_count} mailbox{order.mailbox_count === 1 ? '' : 'es'} ·{' '}
                    {paymentMethodLabel(order.payment_method)}
                  </small>
                </div>
                <span className={order.invoice_status === 'paid' ? 'billing-paid' : ''}>
                  {order.invoice_status === 'paid' && <Check size={13} />}
                  {order.invoice_status === 'paid' ? 'Paid' : order.invoice_status === 'void' ? 'Void' : 'Due'}
                </span>
                <div className="admin-actions">
                  <button
                    type="button"
                    className="secondary-button"
                    onClick={() => navigate(`/mail/billing/invoices/${invoiceId}`)}
                  >
                    <FileText size={13} />
                    View invoice
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
