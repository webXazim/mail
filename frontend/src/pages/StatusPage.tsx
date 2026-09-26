import { useCallback, useEffect, useState } from 'react'
import { Link } from 'react-router-dom'
import { AlertTriangle, ArrowRight, CheckCircle2, RefreshCw } from 'lucide-react'
import { publicApi, type PublicStatus, type PublicStatusComponent } from '../services/public'

const statusBadge = (status: PublicStatusComponent['status']) => {
  if (status === 'operational') {
    return <span className="status-pill status-pill--ok"><CheckCircle2 size={13} />Operational</span>
  }
  if (status === 'degraded') {
    return <span className="status-pill status-pill--warn"><AlertTriangle size={13} />Degraded</span>
  }
  return <span className="status-pill status-pill--error"><AlertTriangle size={13} />Outage</span>
}

const timeFmt = (iso: string) =>
  new Date(iso).toLocaleString([], { month: 'short', day: 'numeric', year: 'numeric', hour: 'numeric', minute: '2-digit' })

export function StatusPage() {
  const [data, setData] = useState<PublicStatus | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState('')

  const load = useCallback(async () => {
    setLoading(true)
    setError('')
    try {
      setData(await publicApi.status())
    } catch (reason) {
      setData(null)
      setError(reason instanceof Error ? reason.message : 'Live status is temporarily unavailable')
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    const timer = window.setTimeout(() => void load(), 0)
    return () => window.clearTimeout(timer)
  }, [load])

  const operational = data?.status === 'operational'
  const headline = data?.status === 'major_outage' ? 'Service interruption' : data?.status === 'degraded' ? 'Some systems degraded' : 'All systems operational'

  return (
    <div className="site-page">
      <section className="site-page__head site-page__head--center">
        <p className="eyebrow">Service status</p>
        <h1>{data ? headline : 'Live system status'}</h1>
        <p className="site-page__intro">
          {data ? `Last checked ${timeFmt(data.updated_at)}.` : 'Status is read directly from CS Mail service health and incident records.'}
        </p>
        <button type="button" className="secondary-button" onClick={() => void load()} disabled={loading}>
          <RefreshCw size={14} /> {loading ? 'Checking…' : 'Refresh'}
        </button>
      </section>

      {error && (
        <div className="status-summary">
          <span className="status-icon status-icon--warn"><AlertTriangle size={24} /></span>
          <strong>Status unavailable</strong>
          <span>{error}. We are not assuming the service is operational while health data is unavailable.</span>
        </div>
      )}

      {data && (
        <>
          <div className="status-summary">
            <span className={`status-icon ${operational ? 'status-icon--ok' : 'status-icon--warn'}`}>
              {operational ? <CheckCircle2 size={24} /> : <AlertTriangle size={24} />}
            </span>
            <strong>{headline}</strong>
            <span>{data.active_incidents === 0 ? 'No active incidents.' : `${data.active_incidents} active incident${data.active_incidents === 1 ? '' : 's'}.`}</span>
          </div>

          <section className="site-section">
            <h2>Components</h2>
            <div className="status-grid">
              {data.components.map((component) => (
                <div className="status-row" key={component.key}>
                  <div><strong>{component.name}</strong><small>Live dependency health</small></div>
                  {statusBadge(component.status)}
                </div>
              ))}
            </div>
          </section>

          <section className="site-section">
            <h2>Incident history</h2>
            {data.incidents.length ? data.incidents.map((incident) => (
              <div className="status-incident" key={incident.id}>
                <div>
                  <strong>{incident.title}</strong>
                  <small>{incident.status} · {incident.impact}{incident.message ? ` · ${incident.message}` : ''}</small>
                </div>
                <span className="status-incident__date">{timeFmt(incident.started_at)}</span>
              </div>
            )) : <p className="settings-hint">No incidents have been recorded in the current history window.</p>}
          </section>
        </>
      )}

      <section className="home-band">
        <div>
          <p className="eyebrow">Need help?</p>
          <h2>Report a problem to <span>CS Mail support</span>.</h2>
        </div>
        <Link to="/contact" className="secondary-button">Report an issue <ArrowRight size={14} /></Link>
      </section>
    </div>
  )
}
