import { useState, type FormEvent } from 'react'
import { Link } from 'react-router-dom'
import { ArrowRight, BadgeCheck, BarChart3, Clock, LifeBuoy, Send } from 'lucide-react'

const topics = ['Billing', 'Security', 'Technical issue', 'Product feedback', 'Press inquiry']

export function ContactPage() {
  const [sent, setSent] = useState(false)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState('')

  const submit = (event: FormEvent) => {
    event.preventDefault()
    const form = new FormData(event.currentTarget as HTMLFormElement)
    if (!String(form.get('name') || '').trim()) {
      setError('Enter your name')
      return
    }
    if (!String(form.get('email') || '').includes('@')) {
      setError('Enter a valid email address')
      return
    }
    if (!String(form.get('message') || '').trim()) {
      setError('Describe how we can help')
      return
    }
    setError('')
    setLoading(true)
    window.setTimeout(() => {
      setLoading(false)
      setSent(true)
    }, 600)
  }

  return (
    <div className="site-page">
      <section className="site-page__head">
        <p className="eyebrow">Support</p>
        <h1>
          Talk to a <span>human</span>
        </h1>
        <p className="site-page__intro">
          We typically reply within one business day. For urgent account issues, include your
          workspace name and the affected email address.
        </p>
      </section>

      {sent ? (
        <div className="contact-success" role="status">
          <span className="auth-status__icon auth-status__icon--success">
            <BadgeCheck size={22} />
          </span>
          <strong>Message sent</strong>
          <p>We&apos;ll reply at the email you provided within one business day.</p>
          <div className="row-actions">
            <Link className="secondary-button" to="/help">
              Back to Help center
            </Link>
            <Link className="primary-button" to="/">
              Return home
              <ArrowRight size={14} />
            </Link>
          </div>
        </div>
      ) : (
        <div className="contact-grid">
          <form className="contact-form" onSubmit={submit}>
            <label>
              Full name
              <input name="name" placeholder="Your name" aria-label="Full name" />
            </label>
            <label>
              Email address
              <input
                name="email"
                type="email"
                autoComplete="email"
                placeholder="you@company.com"
                aria-label="Email address"
              />
            </label>
            <label>
              Topic
              <select name="topic" aria-label="Topic" defaultValue="Billing">
                {topics.map((topic) => (
                  <option key={topic} value={topic}>
                    {topic}
                  </option>
                ))}
              </select>
            </label>
            <label>
              Subject
              <input name="subject" placeholder="Brief description" aria-label="Subject" />
            </label>
            <label>
              Message
              <textarea
                name="message"
                rows={5}
                placeholder="How can we help?"
                aria-label="Message"
              />
            </label>
            {error && (
              <p className="form-error" role="alert">
                {error}
              </p>
            )}
            <div className="row-actions">
              <button className="primary-button" disabled={loading}>
                {loading ? 'Working...' : 'Send message'}
                {!loading && <Send size={14} />}
              </button>
            </div>
          </form>
          <aside className="contact-aside">
            <div className="contact-card">
              <span className="feature-card__icon">
                <Clock size={16} />
              </span>
              <div>
                <strong>Response hours</strong>
                <small>
                  Monday – Friday, 8am – 6pm PT. Security issues are triaged immediately.
                </small>
              </div>
            </div>
            <div className="contact-card">
              <span className="feature-card__icon">
                <BarChart3 size={16} />
              </span>
              <div>
                <strong>System status</strong>
                <small>All systems are operational right now.</small>
                <Link to="/status" className="text-button">
                  Check live status <ArrowRight size={13} />
                </Link>
              </div>
            </div>
            <div className="contact-card">
              <span className="feature-card__icon">
                <LifeBuoy size={16} />
              </span>
              <div>
                <strong>Find an answer faster</strong>
                <small>
                  Browse our help center for setup guides, shortcuts, and troubleshooting.
                </small>
                <Link to="/help" className="text-button">
                  Open Help center <ArrowRight size={13} />
                </Link>
              </div>
            </div>
          </aside>
        </div>
      )}
    </div>
  )
}
