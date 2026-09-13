import { useState } from 'react'
import { Link } from 'react-router-dom'
import { ArrowRight, Check } from 'lucide-react'
import { plans } from '../lib/plans'

const features: Record<string, string[]> = {
  solo: ['1 mailbox', '5 GB storage', 'Harbor domain', 'Community support'],
  team: [
    '5 mailboxes',
    '50 GB per mailbox',
    'Custom domain',
    'Filters & routing',
    'Priority support',
  ],
  business: [
    '25 mailboxes',
    '1 TB per mailbox',
    'Admin center & audit log',
    'Quarantine & security policy',
    'Dedicated support',
  ],
}

export function PublicPricingPage() {
  const [cycle, setCycle] = useState<'monthly' | 'annual'>('monthly')

  const price = (item: { price: string }) => {
    if (cycle === 'monthly') return item.price
    const base = Number(item.price.replace(/[^0-9.]/g, ''))
    return base === 0 ? '$0' : `$${Math.round((base * 10) / 12)}`
  }

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
          Start free, scale when your team does. Every plan includes encryption in transit, 99.9%
          uptime, and a free Harbor domain.
        </p>
      </section>

      <div className="pricing-wrap site-pricing">
        <div className="pricing-toggle" role="group" aria-label="Billing cycle">
          <button
            type="button"
            className={cycle === 'monthly' ? 'pricing-toggle__active' : ''}
            aria-pressed={cycle === 'monthly'}
            onClick={() => setCycle('monthly')}
          >
            Monthly
          </button>
          <button
            type="button"
            className={cycle === 'annual' ? 'pricing-toggle__active' : ''}
            aria-pressed={cycle === 'annual'}
            onClick={() => setCycle('annual')}
          >
            Annual · 2 months free
          </button>
        </div>

        <div className="pricing-grid">
          {plans.map((item) => (
            <section
              className={`pricing-card ${item.id === 'business' ? 'pricing-card--featured' : ''}`}
              key={item.id}
            >
              {item.id === 'business' && <span className="pricing-card__badge">Most popular</span>}
              <p className="pricing-card__name">{item.name}</p>
              <div className="pricing-card__price">
                <strong>{price(item)}</strong>
                <small>/ seat / month</small>
              </div>
              <p className="pricing-card__detail">
                {item.detail}
                {cycle === 'annual' && item.price !== '$0' ? ' · billed annually' : ''}
              </p>
              <ul className="pricing-card__features">
                {features[item.id].map((feature) => (
                  <li key={feature}>
                    <Check size={13} />
                    {feature}
                  </li>
                ))}
              </ul>
              <Link
                to="/login"
                className={item.id === 'business' ? 'primary-button' : 'secondary-button'}
              >
                {item.id === 'solo' ? 'Start free' : `Start with ${item.name}`}
                <ArrowRight size={14} />
              </Link>
            </section>
          ))}
        </div>

        <p className="settings-hint">
          Prices shown are per seat and charged in USD. Cancel anytime — your data stays exportable
          for 30 days.
        </p>
      </div>
    </div>
  )
}
