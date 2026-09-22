import { Link } from 'react-router-dom'
import { ArrowRight, Keyboard, Search, ShieldCheck } from 'lucide-react'

const values = [
  {
    icon: Search,
    title: 'Search that finds',
    body: 'Speak the query, skip the ceremony. from:, is:unread, in:trash — the operators work the way your brain does.',
  },
  {
    icon: Keyboard,
    title: 'Keyboard-first',
    body: 'Every action is a keypress away. j/k to move, e to archive, c to compose, and Ctrl+K for anything else.',
  },
  {
    icon: ShieldCheck,
    title: 'Security by default',
    body: 'TLS in transit, AES-256 at rest, SOC 2 Type II, and a DPA on request. Your mail is your business.',
  },
]

export function HomePage() {
  return (
    <div className="home">
      <section className="home-hero">
        <p className="eyebrow">CS Mail · Business email that respects your focus</p>
        <h1>
          Calm, focused email
          <br />
          for <span>teams</span>.
        </h1>
        <p className="home-hero__copy">
          A premium inbox that keeps important conversations moving without the noise. Fast search,
          keyboard shortcuts, swipe-to-triage, and a reading experience built for serious work.
        </p>
        <div className="home-hero__actions">
          <Link to="/login" className="primary-button">
            Create account <ArrowRight size={15} />
          </Link>
          <Link to="/pricing" className="secondary-button">
            See pricing
          </Link>
        </div>
        <p className="home-hero__note">Business email from SAR 25/month · Custom domain · Web + IMAP/SMTP</p>
      </section>

      <section className="home-value" aria-label="Why CS Mail">
        {values.map((item) => {
          const Icon = item.icon
          return (
            <article className="home-card" key={item.title}>
              <span className="home-card__icon">
                <Icon size={16} />
              </span>
              <h2>{item.title}</h2>
              <p>{item.body}</p>
            </article>
          )
        })}
      </section>

      <section className="home-band">
        <div>
          <p className="eyebrow">Built like a tool, not a widget</p>
          <h2>From inbox zero to shipped, without leaving the keyboard.</h2>
        </div>
        <Link to="/features" className="secondary-button">
          Explore the features <ArrowRight size={14} />
        </Link>
      </section>
    </div>
  )
}
