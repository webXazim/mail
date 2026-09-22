import { useEffect, useState, type FormEvent } from 'react'
import { Link } from 'react-router-dom'
import { ArrowRight, BadgeCheck, BarChart3, Clock, LifeBuoy, Send } from 'lucide-react'
import { supportApi, type SupportTicket } from '../services/support'

const topics = ['Billing', 'Security', 'Technical issue', 'Product feedback', 'Press inquiry', 'Other']

export function ContactPage() {
  const [sent, setSent] = useState(false)
  const [reference, setReference] = useState('')
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState('')
  const [recentTickets, setRecentTickets] = useState<SupportTicket[]>([])

  useEffect(() => {
    let alive = true
    supportApi.mine().then((tickets) => { if (alive) setRecentTickets(tickets) }).catch(() => undefined)
    return () => { alive = false }
  }, [])

  const submit = async (event: FormEvent) => {
    event.preventDefault()
    const form = new FormData(event.currentTarget as HTMLFormElement)
    const name = String(form.get('name') || '').trim()
    const email = String(form.get('email') || '').trim()
    const topic = String(form.get('topic') || '').trim()
    const subject = String(form.get('subject') || '').trim()
    const message = String(form.get('message') || '').trim()
    if (!name) {
      setError('Enter your name')
      return
    }
    if (!email.includes('@')) {
      setError('Enter a valid email address')
      return
    }
    if (!message) {
      setError('Describe how we can help')
      return
    }
    setError('')
    setLoading(true)
    try {
      const result = await supportApi.create({ name, email, topic, subject, message })
      setReference(result.reference)
      setSent(true)
      supportApi.mine().then(setRecentTickets).catch(() => undefined)
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : 'Unable to send your support request')
    } finally {
      setLoading(false)
    }
  }

  return (
    <div className="site-page">
      <section className="site-page__head">
        <p className="eyebrow">Support</p>
        <h1>
          Talk to a <span>human</span>
        </h1>
        <p className="site-page__intro">
          Requests are queued for support review, and security issues are marked high priority automatically.
        </p>
      </section>

      {sent ? (
        <div className="contact-success" role="status">
          <span className="auth-status__icon auth-status__icon--success">
            <BadgeCheck size={22} />
          </span>
          <strong>Support request received</strong>
          <p>
            Your ticket is <strong>{reference}</strong>. We&apos;ll reply to the email you provided.
          </p>
          <div className="row-actions">
            <Link className="secondary-button" to="/help">Back to Help center</Link>
            <Link className="primary-button" to="/">
              Return home <ArrowRight size={14} />
            </Link>
          </div>
        </div>
      ) : (
        <div className="contact-grid">
          <form className="contact-form" onSubmit={(event) => void submit(event)}>
            <label>
              Full name
              <input name="name" placeholder="Your name" aria-label="Full name" autoComplete="name" />
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
              <select name="topic" aria-label="Topic" defaultValue="Technical issue">
                {topics.map((topic) => <option key={topic} value={topic}>{topic}</option>)}
              </select>
            </label>
            <label>
              Subject
              <input name="subject" placeholder="Brief description" aria-label="Subject" maxLength={200} />
            </label>
            <label>
              Message
              <textarea
                name="message"
                rows={5}
                placeholder="How can we help?"
                aria-label="Message"
                maxLength={10000}
              />
            </label>
            {error && <p className="form-error" role="alert">{error}</p>}
            <div className="row-actions">
              <button className="primary-button" disabled={loading}>
                {loading ? 'Sending…' : 'Send message'}
                {!loading && <Send size={14} />}
              </button>
            </div>
          </form>
          <aside className="contact-aside">
            <div className="contact-card">
              <span className="feature-card__icon"><Clock size={16} /></span>
              <div>
                <strong>Support queue</strong>
                <small>Your request receives a durable ticket reference. Security requests are marked high priority for review.</small>
              </div>
            </div>
            <div className="contact-card">
              <span className="feature-card__icon"><BarChart3 size={16} /></span>
              <div>
                <strong>System status</strong>
                <small>See the live health of CS Mail and any active incidents.</small>
                <Link to="/status" className="text-button">Check live status <ArrowRight size={13} /></Link>
              </div>
            </div>
            <div className="contact-card">
              <span className="feature-card__icon"><LifeBuoy size={16} /></span>
              <div>
                <strong>Find an answer faster</strong>
                <small>Browse setup guides, shortcuts, and troubleshooting in the Help center.</small>
                <Link to="/help" className="text-button">Open Help center <ArrowRight size={13} /></Link>
              </div>
            </div>
          </aside>
        </div>
      )}

      {recentTickets.length > 0 && (
        <section className="site-section">
          <h2>Your recent support tickets</h2>
          {recentTickets.slice(0, 5).map((ticket) => (
            <div className="status-incident" key={ticket.id ?? ticket.reference}>
              <div>
                <strong>{ticket.reference} · {ticket.subject}</strong>
                <small>{ticket.status}{ticket.updated_at ? ` · updated ${new Date(ticket.updated_at).toLocaleString()}` : ''}</small>
              </div>
            </div>
          ))}
        </section>
      )}
    </div>
  )
}
