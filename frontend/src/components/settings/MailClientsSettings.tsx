import { useCallback, useEffect, useMemo, useState } from 'react'
import { Check, Copy, KeyRound, RefreshCw, Trash2, Upload } from 'lucide-react'
import {
  mailClientsApi,
  type CreatedAppPassword,
  type MailClientOverview,
  type MailImport,
} from '../../services/mailClients'

function sizeLabel(bytes: number) {
  if (!Number.isFinite(bytes) || bytes <= 0) return '0 B'
  const units = ['B', 'KB', 'MB', 'GB']
  let value = bytes
  let unit = 0
  while (value >= 1024 && unit < units.length - 1) { value /= 1024; unit += 1 }
  return `${value >= 10 || unit === 0 ? value.toFixed(0) : value.toFixed(1)} ${units[unit]}`
}

function statusLabel(status: string) {
  return status.replaceAll('_', ' ').replace(/\b\w/g, (value) => value.toUpperCase())
}

export function MailClientsSettings() {
  const [overview, setOverview] = useState<MailClientOverview | null>(null)
  const [imports, setImports] = useState<MailImport[]>([])
  const [loading, setLoading] = useState(true)
  const [notice, setNotice] = useState('')
  const [error, setError] = useState('')
  const [creating, setCreating] = useState(false)
  const [created, setCreated] = useState<CreatedAppPassword | null>(null)
  const [copied, setCopied] = useState(false)
  const [label, setLabel] = useState('Desktop mail client')
  const [currentPassword, setCurrentPassword] = useState('')
  const [expiresAt, setExpiresAt] = useState('')
  const [allowedIps, setAllowedIps] = useState('')
  const [uploading, setUploading] = useState(false)
  const [file, setFile] = useState<File | null>(null)

  const refresh = useCallback(async () => {
    setError('')
    try {
      const [nextOverview, nextImports] = await Promise.all([
        mailClientsApi.overview(),
        mailClientsApi.imports(),
      ])
      setOverview(nextOverview)
      setImports(nextImports)
    } catch (incoming) {
      setOverview(null)
      setImports([])
      setError(incoming instanceof Error ? incoming.message : 'Could not load mail-client settings')
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    const initialRefresh = window.setTimeout(() => void refresh(), 0)
    const onMailboxChange = () => { setLoading(true); setCreated(null); void refresh() }
    window.addEventListener('cs-mail-mailbox-context-changed', onMailboxChange)
    return () => {
      window.clearTimeout(initialRefresh)
      window.removeEventListener('cs-mail-mailbox-context-changed', onMailboxChange)
    }
  }, [refresh])

  useEffect(() => {
    const pending = imports.some((item) => item.status === 'queued' || item.status === 'running' || item.status === 'retry')
    if (!pending) return
    const timer = window.setInterval(() => void refresh(), 4000)
    return () => window.clearInterval(timer)
  }, [imports, refresh])

  const usablePasswords = useMemo(
    () => overview?.app_passwords.filter((item) => item.status === 'active' || item.status === 'expired') ?? [],
    [overview],
  )

  const create = async () => {
    setError('')
    setNotice('')
    setCreated(null)
    setCreating(true)
    try {
      const allowed = allowedIps.split(/[\s,]+/).map((value) => value.trim()).filter(Boolean)
      const expiry = expiresAt ? new Date(expiresAt).toISOString() : null
      const result = await mailClientsApi.createAppPassword({
        label,
        current_password: currentPassword,
        expires_at: expiry,
        allowed_ips: allowed,
      })
      setCreated(result)
      setCurrentPassword('')
      setNotice('App password created. Copy it before leaving this page.')
      await refresh()
    } catch (incoming) {
      setError(incoming instanceof Error ? incoming.message : 'Could not create app password')
    } finally {
      setCreating(false)
    }
  }

  const copySecret = async () => {
    if (!created) return
    await navigator.clipboard.writeText(created.secret)
    setCopied(true)
    window.setTimeout(() => setCopied(false), 1800)
  }

  const revoke = async (id: string) => {
    setError('')
    try {
      await mailClientsApi.revokeAppPassword(id)
      setNotice('App password revoked.')
      await refresh()
    } catch (incoming) {
      setError(incoming instanceof Error ? incoming.message : 'Could not revoke app password')
    }
  }

  const upload = async () => {
    if (!file) return
    setUploading(true)
    setError('')
    setNotice('')
    try {
      await mailClientsApi.uploadImport(file)
      setFile(null)
      setNotice('Mailbox import queued. You can leave this page while it runs.')
      await refresh()
    } catch (incoming) {
      setError(incoming instanceof Error ? incoming.message : 'Could not upload mailbox archive')
    } finally {
      setUploading(false)
    }
  }

  const cancel = async (id: string) => {
    setError('')
    try {
      await mailClientsApi.cancelImport(id)
      setNotice('Import cancellation requested.')
      await refresh()
    } catch (incoming) {
      setError(incoming instanceof Error ? incoming.message : 'Could not cancel mailbox import')
    }
  }

  if (loading) return <div className="settings-section"><p className="settings-hint">Loading mail-client settings…</p></div>

  return (
    <div>
      {error && <p className="settings-notice">{error}</p>}
      {notice && <p className="settings-notice settings-notice--ok">{notice}</p>}

      <div className="settings-section">
        <h2>Mail clients</h2>
        <p className="settings-hint">
          Connect Thunderbird, Outlook, Apple Mail, phones, and other IMAP/SMTP clients with an app password. Your normal CS Mail website password is never used by those clients.
        </p>
        {overview && (
          <>
            <div className="billing-row"><div><strong>Username</strong><small>Full mailbox address</small></div><span className="admin-record">{overview.config.username}</span></div>
            <div className="billing-row"><div><strong>Incoming mail</strong><small>IMAP over TLS</small></div><span className="admin-record">{overview.config.incoming.host}:{overview.config.incoming.port}</span></div>
            <div className="billing-row"><div><strong>Outgoing mail</strong><small>SMTP submission with {overview.config.outgoing.security}</small></div><span className="admin-record">{overview.config.outgoing.host}:{overview.config.outgoing.port}</span></div>
          </>
        )}
      </div>

      <div className="settings-section">
        <h2>Create app password</h2>
        <label>Client name<input value={label} maxLength={120} onChange={(event) => setLabel(event.target.value)} placeholder="MacBook Mail" /></label>
        <label>Current CS Mail password<input type="password" autoComplete="current-password" value={currentPassword} onChange={(event) => setCurrentPassword(event.target.value)} /></label>
        <label>Expires (optional)<input type="datetime-local" value={expiresAt} onChange={(event) => setExpiresAt(event.target.value)} /></label>
        <label>Allowed IPs / CIDR (optional)<input value={allowedIps} onChange={(event) => setAllowedIps(event.target.value)} placeholder="203.0.113.8, 2001:db8::/48" /></label>
        <p className="settings-hint">Leave allowed IPs empty for roaming/mobile clients. A mailbox can have up to {overview?.max_app_passwords ?? 5} active app passwords.</p>
        <button type="button" className="primary-button" disabled={creating || !label.trim() || !currentPassword} onClick={() => void create()}>
          <KeyRound size={14} /> {creating ? 'Creating…' : 'Create app password'}
        </button>
        {created && (
          <div className="mail-client-secret" role="status">
            <div><strong>Copy this password now</strong><small>It is shown once and is not stored by CS Mail.</small></div>
            <code>{created.secret}</code>
            <button type="button" className="secondary-button" onClick={() => void copySecret()}>{copied ? <Check size={14} /> : <Copy size={14} />}{copied ? 'Copied' : 'Copy'}</button>
          </div>
        )}
      </div>

      <div className="settings-section">
        <h2>Active app passwords</h2>
        {usablePasswords.length === 0 && <p className="settings-hint">No active external mail-client credentials.</p>}
        {usablePasswords.map((item) => (
          <div className="billing-row" key={item.id}>
            <div><strong>{item.label}</strong><small>{statusLabel(item.status)} · created {new Date(item.created_at).toLocaleString()}{item.expires_at ? ` · expires ${new Date(item.expires_at).toLocaleString()}` : ' · no expiry'}</small></div>
            <button type="button" className="secondary-button" onClick={() => void revoke(item.id)}><Trash2 size={14} /> Revoke</button>
          </div>
        ))}
      </div>

      <div className="settings-section">
        <h2>Import mailbox</h2>
        <p className="settings-hint">Import a standard MBOX export. The archive is streamed to durable storage and processed in the background; source IMAP passwords are never stored.</p>
        <label>MBOX archive<input type="file" accept=".mbox,application/mbox,application/octet-stream" onChange={(event) => setFile(event.target.files?.[0] ?? null)} /></label>
        {file && <p className="settings-hint">{file.name} · {sizeLabel(file.size)}</p>}
        <button type="button" className="primary-button" disabled={!file || uploading} onClick={() => void upload()}><Upload size={14} /> {uploading ? 'Uploading…' : 'Upload and import'}</button>
      </div>

      <div className="settings-section">
        <div className="mail-client-section-head"><h2>Import history</h2><button type="button" className="secondary-button" onClick={() => void refresh()}><RefreshCw size={14} /> Refresh</button></div>
        {imports.length === 0 && <p className="settings-hint">No mailbox imports yet.</p>}
        {imports.map((item) => (
          <div className="billing-row" key={item.id}>
            <div>
              <strong>{item.original_filename}</strong>
              <small>{statusLabel(item.status)} · {item.imported_messages}/{item.total_messages || '?'} imported · {item.failed_messages} failed · {sizeLabel(item.byte_size)}</small>
              {item.last_error && <small>{item.last_error}</small>}
            </div>
            {(item.status === 'queued' || item.status === 'running' || item.status === 'retry') && <button type="button" className="secondary-button" onClick={() => void cancel(item.id)}>Cancel</button>}
          </div>
        ))}
      </div>
    </div>
  )
}
