import { useEffect, useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { ArrowLeft, Check, CreditCard } from 'lucide-react'
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

export function PricingPage() {
  const navigate = useNavigate()
  const [cycle, setCycle] = useState<'monthly' | 'annual'>('monthly')

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') navigate('/mail/inbox')
    }
    window.addEventListener('keydown', handleKeyDown)
    return () => window.removeEventListener('keydown', handleKeyDown)
  }, [navigate])

  const price = (item: { price: string }) => {
    if (cycle === 'monthly') return item.price
    const base = Number(item.price.replace(/[^0-9.]/g, ''))
    return base === 0 ? '$0' : `$${Math.round((base * 10) / 12)}`
  }

  return (
    <div className="pricing-page settings-page" role="region" aria-label="Pricing">
      <header className="calendar-head">
        <div>
          <p className="eyebrow">Harbor Mail</p>
          <h1>Pricing</h1>
        </div>
        <div className="calendar-head__actions">
          <button
            type="button"
            className="secondary-button"
            onClick={() => navigate('/mail/billing')}
          >
            <CreditCard size={14} />
            View billing
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

      <div className="pricing-wrap">
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
              <button
                type="button"
                className={item.id === 'business' ? 'primary-button' : 'secondary-button'}
                onClick={() => navigate('/mail/billing')}
              >
                {item.id === 'solo' ? 'Start free' : `Choose ${item.name}`}
              </button>
            </section>
          ))}
        </div>

        <p className="settings-hint">
          Every plan includes end-to-end encryption in transit, 99.9% uptime, and a free Harbor
          domain. Prices shown are per seat and charged in USD. Cancel anytime.
        </p>
      </div>
    </div>
  )
}
