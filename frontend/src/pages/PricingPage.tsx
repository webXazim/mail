import { useCallback, useEffect, useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { ArrowLeft, Check, CreditCard, RefreshCw } from 'lucide-react'
import { publicApi, type PublicPlan } from '../services/public'

const money = (cents: number, currency: string) =>
  new Intl.NumberFormat(undefined, {
    style: 'currency',
    currency: currency || 'SAR',
    maximumFractionDigits: cents % 100 === 0 ? 0 : 2,
  }).format(cents / 100)

const bytes = (value: number) => {
  const gb = value / 1024 / 1024 / 1024
  return `${Number.isInteger(gb) ? gb : gb.toFixed(1)} GB`
}

export function PricingPage() {
  const navigate = useNavigate()
  const [plans, setPlans] = useState<PublicPlan[]>([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState('')

  const load = useCallback(() => {
    setLoading(true)
    setError('')
    publicApi
      .plans()
      .then(setPlans)
      .catch((reason) => setError(reason instanceof Error ? reason.message : 'Pricing is temporarily unavailable'))
      .finally(() => setLoading(false))
  }, [])

  useEffect(() => {
    const initialLoad = window.setTimeout(load, 0)
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') navigate('/mail/inbox')
    }
    window.addEventListener('keydown', handleKeyDown)
    return () => {
      window.clearTimeout(initialLoad)
      window.removeEventListener('keydown', handleKeyDown)
    }
  }, [load, navigate])

  return (
    <div className="pricing-page settings-page" role="region" aria-label="Pricing">
      <header className="calendar-head">
        <div>
          <p className="eyebrow">CS Mail</p>
          <h1>Pricing</h1>
        </div>
        <div className="calendar-head__actions">
          <button type="button" className="secondary-button" onClick={load} disabled={loading}>
            <RefreshCw size={14} />
            Refresh
          </button>
          <button type="button" className="secondary-button" onClick={() => navigate('/mail/billing')}>
            <CreditCard size={14} />
            View billing
          </button>
          <button type="button" className="secondary-button" onClick={() => navigate('/mail/inbox')}>
            <ArrowLeft size={14} />
            Back to inbox
          </button>
        </div>
      </header>

      <div className="pricing-wrap">
        {loading && (
          <div className="list-state">
            <div className="loading-spinner" />
            <span>Loading plans…</span>
          </div>
        )}
        {error && <p className="form-error" role="alert">{error}</p>}

        {!loading && !error && (
          <div className="pricing-grid">
            {plans.map((item) => (
              <section
                className={`pricing-card ${item.code === 'team' ? 'pricing-card--featured' : ''}`}
                key={item.code}
              >
                {item.code === 'team' && <span className="pricing-card__badge">Popular</span>}
                <p className="pricing-card__name">{item.name}</p>
                <div className="pricing-card__price">
                  <strong>{money(item.price_cents, item.currency)}</strong>
                  <small>/ {item.interval}</small>
                </div>
                <p className="pricing-card__detail">
                  {item.mailbox_limit} {item.mailbox_limit === 1 ? 'mailbox' : 'mailboxes'} included · {bytes(item.mailbox_bytes)} per mailbox · {item.domain_limit} {item.domain_limit === 1 ? 'domain' : 'domains'}
                </p>
                <ul className="pricing-card__features">
                  {(item.features ?? []).map((feature) => (
                    <li key={feature}>
                      <Check size={13} />
                      {feature}
                    </li>
                  ))}
                  <li><Check size={13} />Additional mailbox: {money(item.extra_mailbox_price_cents, item.currency)} / {item.interval}</li>
                  <li><Check size={13} />{item.alias_limit_per_mailbox === null ? 'Unlimited aliases per mailbox' : `${item.alias_limit_per_mailbox} aliases per mailbox`}</li>
                  <li><Check size={13} />Up to {item.max_mailboxes} mailboxes by self-service</li>
                  <li><Check size={13} />Up to {item.max_recipients} recipients per message</li>
                </ul>
                <button
                  type="button"
                  className={item.code === 'team' ? 'primary-button' : 'secondary-button'}
                  onClick={() => navigate(`/mail/billing?plan=${encodeURIComponent(item.code)}`)}
                >
                  {`Choose ${item.name.replace(/^CS Mail\s+/i, '')}`}
                </button>
              </section>
            ))}
          </div>
        )}

        {!loading && !error && plans.length === 0 && (
          <p className="settings-hint">No plans are currently available.</p>
        )}
        <p className="settings-hint">
          Prices, billing intervals, quotas, and feature limits come from the same plan records used to enforce your account entitlements. VAT is added only when applicable and configured by the seller.
        </p>
      </div>
    </div>
  )
}
