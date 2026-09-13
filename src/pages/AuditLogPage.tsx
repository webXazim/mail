import { useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { ArrowLeft, History as HistoryIcon, Trash2 } from 'lucide-react'
import { auditApi, type AccountAuditCategory } from '../services/audit'

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
  general: 'General',
}

export function AuditLogPage() {
  const navigate = useNavigate()
  const [entries, setEntries] = useState(() => auditApi.list())
  const [category, setCategory] = useState<'all' | AccountAuditCategory>('all')
  const [cleared, setCleared] = useState(false)
  const visible =
    category === 'all' ? entries : entries.filter((entry) => entry.category === category)

  const clear = () => {
    auditApi.clear()
    setEntries([])
    setCleared(true)
  }

  return (
    <div className="settings-page" role="region" aria-label="Audit log">
      <header className="calendar-head">
        <div>
          <p className="eyebrow">Account / Activity</p>
          <h1>Audit log</h1>
        </div>
        <div className="calendar-head__actions">
          <button
            type="button"
            className="secondary-button"
            onClick={clear}
            disabled={entries.length === 0}
          >
            <Trash2 size={14} />
            Clear log
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

      {cleared ? (
        <div className="list-state">
          <HistoryIcon size={26} />
          <strong>Log cleared</strong>
          <span>Your activity log has been wiped.</span>
          <button
            type="button"
            className="primary-button"
            onClick={() => navigate('/mail/settings')}
          >
            Back to settings
          </button>
        </div>
      ) : (
        <div className="settings-section">
          <div className="admin-section-head">
            <h2>Activity</h2>
            <select
              aria-label="Filter audit log"
              value={category}
              onChange={(event) => setCategory(event.target.value as typeof category)}
            >
              <option value="all">All activity</option>
              {Object.entries(filterLabel).map(([value, label]) => (
                <option key={value} value={value}>
                  {label}
                </option>
              ))}
            </select>
          </div>
          {visible.map((entry) => (
            <div className="billing-row" key={entry.id}>
              <div>
                <strong>{entry.action}</strong>
                <small>{entry.detail}</small>
              </div>
              <span className="admin-audit-time">{timeFmt(entry.time)}</span>
            </div>
          ))}
          {visible.length === 0 && <p className="settings-hint">No activity recorded yet.</p>}
        </div>
      )}
    </div>
  )
}
