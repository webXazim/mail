import { useEffect, useState } from 'react'
import { Link } from 'react-router-dom'
import { ArrowRight, Check } from 'lucide-react'
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

export function PublicPricingPage() {
  const [plans, setPlans] = useState<PublicPlan[]>([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState('')

  useEffect(() => {
    let alive = true
    publicApi
      .plans()
      .then((rows) => {
        if (alive) setPlans(rows)
      })
      .catch((reason) => {
        if (alive) setError(reason instanceof Error ? reason.message : 'Pricing is temporarily unavailable')
      })
      .finally(() => {
        if (alive) setLoading(false)
      })
    return () => {
      alive = false
    }
  }, [])

  return (
    <div className="site-page">
      <section className="site-page__head">
        <p className="eyebrow">Pricing</p>
        <h1>
          Simple pricing.
          <br />
          Serious email.
        </h1>
        <p className="site-page__intro">
          Plans and limits below come directly from the CS Mail billing service, so the public page
          matches the entitlements enforced after signup.
        </p>
      </section>

      <div className="pricing-wrap site-pricing">
        {loading && <div className="list-state"><div className="loading-spinner" /><span>Loading plans…</span></div>}
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
                    <li key={feature}><Check size={13} />{feature}</li>
                  ))}
                  <li><Check size={13} />Additional mailbox: {money(item.extra_mailbox_price_cents, item.currency)} / {item.interval}</li>
                  <li><Check size={13} />{item.alias_limit_per_mailbox === null ? 'Unlimited aliases per mailbox' : `${item.alias_limit_per_mailbox} aliases per mailbox`}</li>
                  <li><Check size={13} />Up to {item.max_mailboxes} mailboxes by self-service</li>
                  <li><Check size={13} />Up to {item.max_recipients} recipients per message</li>
                </ul>
                <Link
                  to="/create-account"
                  className={item.code === 'team' ? 'primary-button' : 'secondary-button'}
                >
                  {`Choose ${item.name.replace(/^CS Mail\s+/i, '')}`}
                  <ArrowRight size={14} />
                </Link>
              </section>
            ))}
          </div>
        )}

        {!loading && !error && plans.length === 0 && (
          <p className="settings-hint">No public plans are currently available.</p>
        )}
        <p className="settings-hint">
          Prices, billing intervals, quotas, and feature limits are served from the same plan records used by the application. VAT is added only when applicable and configured by the seller.
        </p>
      </div>
    </div>
  )
}
