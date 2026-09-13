import { Link } from 'react-router-dom'
import { AlertTriangle, ArrowRight, CheckCircle2 } from 'lucide-react'

const components = [
  { name: 'Email delivery', latency: '34 ms', uptime: '99.99%', status: 'operational' as const },
  { name: 'API & sync', latency: '52 ms', uptime: '99.98%', status: 'operational' as const },
  {
    name: 'Calendar & scheduling',
    latency: '28 ms',
    uptime: '100.00%',
    status: 'operational' as const,
  },
  {
    name: 'Attachments & storage',
    latency: '41 ms',
    uptime: '99.97%',
    status: 'operational' as const,
  },
  { name: 'Billing', latency: '39 ms', uptime: '100.00%', status: 'operational' as const },
  {
    name: 'Sign-in & identity',
    latency: '46 ms',
    uptime: '99.99%',
    status: 'operational' as const,
  },
]

const incidents = [
  {
    date: 'August 21, 2026',
    title: 'Intermittent API latency in US-EAST',
    duration: '42 min',
    root: 'Upstream database failover',
  },
  {
    date: 'July 10, 2026',
    title: 'Calendar sync delay',
    duration: '1 hour 12 min',
    root: 'ICAL feed provider outage',
  },
  {
    date: 'June 3, 2026',
    title: 'Attachment downloads slow',
    duration: '28 min',
    root: 'CDN cache invalidation backlog',
  },
]

const statusBadge = (status: 'operational' | 'degraded' | 'incident') => {
  if (status === 'operational')
    return (
      <span className="status-pill status-pill--ok">
        <CheckCircle2 size={13} />
        Operational
      </span>
    )
  if (status === 'degraded')
    return (
      <span className="status-pill status-pill--warn">
        <AlertTriangle size={13} />
        Degraded
      </span>
    )
  return (
    <span className="status-pill status-pill--error">
      <AlertTriangle size={13} />
      Incident
    </span>
  )
}

export function StatusPage() {
  return (
    <div className="site-page">
      <section className="site-page__head site-page__head--center">
        <p className="eyebrow">Service status</p>
        <h1>
          All systems <span>operational</span>
        </h1>
        <p className="site-page__intro">
          30-day uptime 99.99% · Last updated September 11, 2026 at 12:00 PM UTC.
        </p>
      </section>

      <div className="status-summary">
        <span className="status-icon status-icon--ok">
          <CheckCircle2 size={24} />
        </span>
        <strong>No active incidents</strong>
        <span>Our team monitors performance around the clock.</span>
      </div>

      <section className="site-section">
        <h2>Components</h2>
        <div className="status-grid">
          {components.map((component) => (
            <div className="status-row" key={component.name}>
              <div>
                <strong>{component.name}</strong>
                <small>
                  {component.latency} · 30-day uptime {component.uptime}
                </small>
              </div>
              {statusBadge(component.status)}
            </div>
          ))}
        </div>
      </section>

      <section className="site-section">
        <h2>Past incidents</h2>
        {incidents.map((incident) => (
          <div className="status-incident" key={incident.title}>
            <div>
              <strong>{incident.title}</strong>
              <small>
                {incident.duration} · Resolved · {incident.root}
              </small>
            </div>
            <span className="status-incident__date">{incident.date}</span>
          </div>
        ))}
      </section>

      <section className="home-band">
        <div>
          <p className="eyebrow">Incidents are rare</p>
          <h2>
            When they happen, we <span>tell you</span> — honestly and fast.
          </h2>
        </div>
        <Link to="/contact" className="secondary-button">
          Report an issue <ArrowRight size={14} />
        </Link>
      </section>
    </div>
  )
}
