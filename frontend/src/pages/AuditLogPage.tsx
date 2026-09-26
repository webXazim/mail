import { useCallback, useEffect, useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { ArrowLeft, History as HistoryIcon } from 'lucide-react'
import { auditApi, type AccountAuditCategory, type AccountAuditEntry } from '../services/audit'

const timeFmt = (iso: string) =>
  new Date(iso).toLocaleString([], {
    month: 'short',
    day: 'numeric',
    hour: 'numeric',
    minute: '2-digit',
  })

const filterLabel: Record<AccountAuditCategory, string> = {
  'sign-in': 'Sign-ins',
  security: 'Security',
  billing: 'Billing',
  mail: 'Mail',
  general: 'General',
}

export function AuditLogPage() {
  const navigate = useNavigate()
  const [entries, setEntries] = useState<AccountAuditEntry[]>(() => auditApi.list())
  const [category, setCategory] = useState<'all' | AccountAuditCategory>('all')
  const [nextBefore, setNextBefore] = useState<string | null>(null)
  const [hasMore, setHasMore] = useState(false)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState('')

  const load = useCallback(async (append = false) => {
    try {
      setError('')
      const page = await auditApi.page(category, append ? nextBefore : null)
      setEntries((current) => (append ? [...current, ...page.entries] : page.entries))
      setHasMore(page.hasMore)
      setNextBefore(page.nextBefore)
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : 'Unable to load account activity')
    } finally {
      setLoading(false)
    }
  }, [category, nextBefore])

  useEffect(() => {
    let active = true
    void auditApi.page(category, null)
      .then((page) => {
        if (!active) return
        setEntries(page.entries)
        setHasMore(page.hasMore)
        setNextBefore(page.nextBefore)
      })
      .catch((reason) => {
        if (active) setError(reason instanceof Error ? reason.message : 'Unable to load account activity')
      })
      .finally(() => {
        if (active) setLoading(false)
      })
    return () => { active = false }
  }, [category])

  const changeCategory = (next: 'all' | AccountAuditCategory) => {
    setLoading(true)
    setError('')
    setNextBefore(null)
    setCategory(next)
  }

  return (
    <div className="settings-page" role="region" aria-label="Audit log">
      <header className="calendar-head">
        <div>
          <p className="eyebrow">Account / Activity</p>
          <h1>Activity log</h1>
        </div>
        <button type="button" className="secondary-button" onClick={() => navigate('/mail/inbox')}>
          <ArrowLeft size={14} />
          Back to inbox
        </button>
      </header>

      <div className="settings-section">
        <div className="admin-section-head">
          <div>
            <h2>Account activity</h2>
            <p className="settings-hint">Security and account history is retained server-side and cannot be cleared from the browser.</p>
          </div>
          <select
            aria-label="Filter activity log"
            value={category}
            onChange={(event) => changeCategory(event.target.value as typeof category)}
          >
            <option value="all">All activity</option>
            {Object.entries(filterLabel).map(([value, label]) => (
              <option key={value} value={value}>{label}</option>
            ))}
          </select>
        </div>

        {error && <p className="form-error" role="alert">{error}</p>}
        {loading && entries.length === 0 ? (
          <div className="list-state"><div className="loading-spinner" /><span>Loading activity…</span></div>
        ) : entries.length ? (
          <>
            {entries.map((entry) => (
              <div className="billing-row" key={entry.id}>
                <div>
                  <strong>{entry.action}</strong>
                  {entry.detail && <small>{entry.detail}</small>}
                </div>
                <span className="admin-audit-time">{timeFmt(entry.time)}</span>
              </div>
            ))}
            {hasMore && (
              <button type="button" className="secondary-button" onClick={() => void load(true)}>
                Load older activity
              </button>
            )}
          </>
        ) : (
          <div className="list-state">
            <HistoryIcon size={26} />
            <strong>No activity in this category</strong>
            <span>New account events will appear here automatically.</span>
          </div>
        )}
      </div>
    </div>
  )
}
