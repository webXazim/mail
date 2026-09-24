import { useEffect, useState, type FormEvent } from 'react'
import { useNavigate, useSearchParams } from 'react-router-dom'
import { billingApi, formatPrice, friendlyError, type PlanCatalog, type PlanView } from '../services/billing'
import { organizationsApi } from '../services/organizations'
import { profileApi } from '../services/profile'

/** The commercial entry point for a verified login without a business. */
export function PlanOnboardingPage() {
  const navigate = useNavigate()
  const [params] = useSearchParams()
  const [plans, setPlans] = useState<PlanView[]>([])
  const [onboarding, setOnboarding] = useState<PlanCatalog['onboarding'] | null>(null)
  const [selected, setSelected] = useState(params.get('plan') || '')
  const [businessName, setBusinessName] = useState('')
  const [method, setMethod] = useState('bank')
  const [loading, setLoading] = useState(true)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')

  useEffect(() => {
    let alive = true
    billingApi.catalog()
      .then(({ plans: rows, onboarding: state }) => {
        if (!alive) return
        setOnboarding(state)
        const active = rows.filter((row) => row.active)
        setPlans(active)
        setSelected((value) => active.some((row) => row.code === value) ? value : active[0]?.code || '')
      })
      .catch((cause) => { if (alive) setError(friendlyError(cause)) })
      .finally(() => { if (alive) setLoading(false) })
    return () => { alive = false }
  }, [])

  const plan = plans.find((row) => row.code === selected)
  const pausedStep = onboarding && (!onboarding.business_creation_enabled
    ? 'New business creation is paused.'
    : !onboarding.plan_ordering_enabled ? 'New plan orders are paused.' : '')
  const start = async (event: FormEvent) => {
    event.preventDefault()
    if (!plan || !businessName.trim() || pausedStep) return
    setBusy(true)
    setError('')
    try {
      // A business is a billing identity. It remains suspended until the
      // chosen order is activated; no domain or mailbox is provisioned here.
      const business = await organizationsApi.create(businessName.trim())
      await organizationsApi.activate(business.id)
      await billingApi.createOrder(plan.code, plan.mailbox_limit, method, '')
      await profileApi.refresh()
      navigate('/mail/billing', { replace: true })
    } catch (cause) {
      setError(friendlyError(cause))
      // If business creation succeeded but ordering failed, the next reload
      // opens Billing for that business so the order can be retried.
      await profileApi.refresh().catch(() => undefined)
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="settings-page" role="region" aria-label="Choose a business mail plan">
      <header className="calendar-head"><div><p className="eyebrow">Step 1 of 3</p><h1>Choose your mail plan</h1><p>Select a plan and add your business name. Domain setup starts after activation.</p></div></header>
      {loading ? <div className="route-loader"><div className="loading-spinner" /></div> : (
        <form className="settings-section" onSubmit={(event) => void start(event)}>
          {plans.length === 0 && !error && <p>No plans are available right now.</p>}
          {plans.map((item) => (
            <button key={item.code} type="button" className={`plan-option ${selected === item.code ? 'plan-option--active' : ''}`} onClick={() => setSelected(item.code)} aria-pressed={selected === item.code}>
              <span><strong>{item.name}</strong><small>{item.mailbox_limit} mailbox{item.mailbox_limit === 1 ? '' : 'es'} · {item.domain_limit} domain{item.domain_limit === 1 ? '' : 's'} · {item.features.join(' · ')}</small></span>
              <span className="plan-option__price"><strong>{formatPrice(item.price_cents, item.currency)}</strong><small>per {item.interval}</small></span>
            </button>
          ))}
          {plan && <>
            <label>Business name<input required maxLength={120} value={businessName} onChange={(event) => setBusinessName(event.target.value)} placeholder="Your company name" /></label>
            <label>Payment method<select value={method} onChange={(event) => setMethod(event.target.value)}><option value="bank">Bank transfer</option><option value="other">Other manual payment</option></select></label>
            <p className="settings-hint">{onboarding?.instant_activation
              ? 'Testing mode: placing this order activates the plan immediately and still issues an unpaid invoice. Use only for isolated acceptance testing.'
              : 'Placing the order issues an invoice. Your plan becomes active after payment approval. You can then verify your domain and create mailboxes.'}</p>
            {pausedStep && <p className="form-error" role="status">{pausedStep} {onboarding?.maintenance_message}</p>}
            <button className="primary-button" disabled={busy || !businessName.trim() || Boolean(pausedStep)}>{busy ? 'Creating order…' : onboarding?.instant_activation ? 'Place test order' : 'Continue to invoice'}</button>
          </>}
          {error && <p className="form-error" role="alert">{error}</p>}
        </form>
      )}
    </div>
  )
}
