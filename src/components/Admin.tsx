import { useRef, useState, type FormEvent } from 'react'
import { AtSign, Check, Copy, Globe, Mailbox, RefreshCw, ShieldCheck, Trash2, X } from 'lucide-react'
import { useFocusTrap } from '../hooks/useFocusTrap'
import { adminApi, dnsRecords, type Alias, type MailboxAccount } from '../services/admin'
import { identitiesApi } from '../services/identities'

type Tab = 'mailboxes' | 'aliases' | 'forwarders' | 'domain'

const statusLabel: Record<string, string> = { active: 'Active', quarantine: 'Quarantine', disabled: 'Disabled' }

export function Admin({ close }: { close: () => void }) {
  const panelRef = useRef<HTMLElement>(null)
  useFocusTrap(panelRef)
  const [tab, setTab] = useState<Tab>('mailboxes')
  const [mailboxes, setMailboxes] = useState<MailboxAccount[]>(() => adminApi.listMailboxes())
  const [aliases, setAliases] = useState<Alias[]>(() => adminApi.listAliases())
  const [forwarders, setForwarders] = useState(() => adminApi.listForwarders())
  const [dns, setDns] = useState(() => adminApi.getDns())
  const [domain, setDomain] = useState(() => adminApi.getDomainSettings())

  const [mailboxForm, setMailboxForm] = useState({ email: '', displayName: '' })
  const [aliasForm, setAliasForm] = useState({ local: '', forwardTo: '' })
  const [forwarderForm, setForwarderForm] = useState({ from: mailboxes[0]?.email ?? '', to: '' })
  const [notice, setNotice] = useState('')
  const [copied, setCopied] = useState<keyof typeof dns | ''>('')

  const showNotice = (message: string) => {
    setNotice(message)
    window.setTimeout(() => setNotice(''), 3500)
  }

  const addMailbox = (event: FormEvent) => {
    event.preventDefault()
    const raw = mailboxForm.email.trim().toLowerCase()
    const full = raw.includes('@') ? raw : `${raw}@harbor.co`
    const current = adminApi.addMailbox({ email: raw, displayName: mailboxForm.displayName })
    if (current === mailboxes) { showNotice('That mailbox already exists'); return }
    setMailboxes(current)
    identitiesApi.add({ email: raw, displayName: mailboxForm.displayName })
    setMailboxForm({ email: '', displayName: '' })
    showNotice(`Mailbox ${full} created`)
  }

  const removeMailbox = (id: string) => {
    const target = mailboxes.find(mailbox => mailbox.id === id)
    const next = adminApi.removeMailbox(id)
    if (next === mailboxes) { showNotice('The primary mailbox can’t be removed'); return }
    setMailboxes(next)
    setForwarderForm(current => ({ ...current, from: current.from === target?.email ? (next[0]?.email ?? '') : current.from }))
    showNotice('Mailbox removed')
  }

  const addAlias = (event: FormEvent) => {
    event.preventDefault()
    if (!aliasForm.forwardTo) return
    const next = adminApi.addAlias(aliasForm.local, aliasForm.forwardTo, domain.domain)
    if (next === aliases) { showNotice('That alias already exists'); return }
    setAliases(next)
    setAliasForm({ local: '', forwardTo: '' })
    showNotice(`Alias ${aliasForm.local.trim().toLowerCase()}@${domain.domain} created`)
  }

  const removeAlias = (id: string) => {
    setAliases(adminApi.removeAlias(id))
  }

  const addForwarder = (event: FormEvent) => {
    event.preventDefault()
    const next = adminApi.addForwarder(forwarderForm.from, forwarderForm.to)
    if (next === forwarders) { showNotice('That forwarder already exists or the address is invalid'); return }
    setForwarders(next)
    setForwarderForm(current => ({ ...current, to: '' }))
    showNotice('Forwarder created')
  }

  const toggleForwarder = (id: string) => setForwarders(adminApi.toggleForwarder(id))
  const removeForwarder = (id: string) => setForwarders(adminApi.removeForwarder(id))

  const verifyRecords = () => {
    setDns(adminApi.verifyAll())
    showNotice('All records verified')
  }

  const copyRecord = async (record: { name: string; value: string }) => {
    try {
      await navigator.clipboard.writeText(record.value)
      setCopied(record.name as keyof typeof dns)
      window.setTimeout(() => setCopied(''), 1800)
    } catch {
      showNotice('Copy not available in this browser')
    }
  }

  const setCatchAllEnabled = (enabled: boolean) => {
    const next = { ...domain, catchAllEnabled: enabled, catchAll: enabled ? domain.catchAll || mailboxes[0]?.email || '' : domain.catchAll }
    setDomain(next)
    adminApi.saveDomainSettings(next)
  }

  const setCatchAllTarget = (target: string) => {
    const next = { ...domain, catchAll: target, catchAllEnabled: true }
    setDomain(next)
    adminApi.saveDomainSettings(next)
  }

  return (
    <div className="settings-layer" role="presentation">
      <section ref={panelRef} className="settings-panel" role="dialog" aria-modal="true" aria-labelledby="admin-title">
        <header>
          <div><p className="eyebrow">Harbor Mail</p><h2 id="admin-title">Admin panel</h2></div>
          <button type="button" className="icon-button" aria-label="Close admin" onClick={close}><X size={17} /></button>
        </header>
        <nav className="settings-nav" aria-label="Admin sections">
          <button type="button" className={tab === 'mailboxes' ? 'settings-nav--active' : ''} aria-current={tab === 'mailboxes' ? 'page' : undefined} onClick={() => setTab('mailboxes')}><Mailbox size={15} />Mailboxes</button>
          <button type="button" className={tab === 'aliases' ? 'settings-nav--active' : ''} aria-current={tab === 'aliases' ? 'page' : undefined} onClick={() => setTab('aliases')}><AtSign size={15} />Aliases</button>
          <button type="button" className={tab === 'forwarders' ? 'settings-nav--active' : ''} aria-current={tab === 'forwarders' ? 'page' : undefined} onClick={() => setTab('forwarders')}><ShieldCheck size={15} />Forwarders</button>
          <button type="button" className={tab === 'domain' ? 'settings-nav--active' : ''} aria-current={tab === 'domain' ? 'page' : undefined} onClick={() => setTab('domain')}><Globe size={15} />Domain</button>
        </nav>
        <div className="billing-body">
          {notice && <p className="settings-notice settings-notice--ok">{notice}</p>}

          {tab === 'mailboxes' && (
            <>
              <div className="settings-section">
                <h3>Mailboxes</h3>
                {mailboxes.map(mailbox => (
                  <div className="billing-row" key={mailbox.id}>
                    <div><strong>{mailbox.email}</strong><small>{mailbox.displayName} · {mailbox.storageUsedGB.toFixed(1)} GB used</small></div>
                    <span className={mailbox.status === 'active' ? 'billing-paid' : ''}>{mailbox.status === 'active' && <Check size={13} />}{statusLabel[mailbox.status]}</span>
                    <button type="button" className="icon-button" aria-label={`Remove ${mailbox.email}`} onClick={() => removeMailbox(mailbox.id)}><Trash2 size={15} /></button>
                  </div>
                ))}
              </div>
              <form className="settings-section" onSubmit={addMailbox}>
                <h3>Add a mailbox</h3>
                <label>Email address<input value={mailboxForm.email} onChange={event => setMailboxForm(current => ({ ...current, email: event.target.value }))} placeholder="sales" aria-label="Mailbox address" /></label>
                <label>Display name<input value={mailboxForm.displayName} onChange={event => setMailboxForm(current => ({ ...current, displayName: event.target.value }))} placeholder="Sales Team" aria-label="Mailbox display name" /></label>
                <div className="row-actions">
                  <button type="submit" className="primary-button"><Mailbox size={15} />Add mailbox</button>
                </div>
              </form>
            </>
          )}

          {tab === 'aliases' && (
            <>
              <div className="settings-section">
                <h3>Aliases</h3>
                {aliases.map(alias => (
                  <div className="billing-row" key={alias.id}>
                    <div><strong>{alias.address}</strong><small>Forwards to {alias.forwardTo}</small></div>
                    <button type="button" className="icon-button" aria-label={`Remove ${alias.address}`} onClick={() => removeAlias(alias.id)}><Trash2 size={15} /></button>
                  </div>
                ))}
                {aliases.length === 0 && <p className="settings-hint">No aliases yet — add one below.</p>}
              </div>
              <form className="settings-section" onSubmit={addAlias}>
                <h3>Add an alias</h3>
                <label>Address<input value={aliasForm.local} onChange={event => setAliasForm(current => ({ ...current, local: event.target.value }))} placeholder={`name@${domain.domain}`} aria-label="Alias address" /></label>
                <label>Deliver to
                  <select value={aliasForm.forwardTo} onChange={event => setAliasForm(current => ({ ...current, forwardTo: event.target.value }))} aria-label="Alias target">
                    <option value="">Choose a mailbox…</option>
                    {mailboxes.filter(mailbox => mailbox.status === 'active').map(mailbox => <option key={mailbox.id} value={mailbox.email}>{mailbox.email}</option>)}
                  </select>
                </label>
                <div className="row-actions">
                  <button type="submit" className="primary-button" disabled={!aliasForm.forwardTo}><AtSign size={15} />Add alias</button>
                </div>
              </form>
            </>
          )}

          {tab === 'forwarders' && (
            <>
              <div className="settings-section">
                <h3>Forwarders</h3>
                {forwarders.map(forwarder => (
                  <div className="billing-row" key={forwarder.id}>
                    <div><strong>{forwarder.from}</strong><small>→ {forwarder.to}</small></div>
                    <span className={forwarder.enabled ? 'billing-paid' : ''}>{forwarder.enabled && <Check size={13} />}{forwarder.enabled ? 'Enabled' : 'Paused'}</span>
                    <button type="button" className="secondary-button" onClick={() => toggleForwarder(forwarder.id)}>{forwarder.enabled ? 'Pause' : 'Enable'}</button>
                    <button type="button" className="icon-button" aria-label={`Remove forwarder ${forwarder.from}`} onClick={() => removeForwarder(forwarder.id)}><Trash2 size={15} /></button>
                  </div>
                ))}
                {forwarders.length === 0 && <p className="settings-hint">No forwarders yet — add one below.</p>}
              </div>
              <form className="settings-section" onSubmit={addForwarder}>
                <h3>Add a forwarder</h3>
                <label>Mailbox
                  <select value={forwarderForm.from} onChange={event => setForwarderForm(current => ({ ...current, from: event.target.value }))} aria-label="Forwarder mailbox">
                    {mailboxes.filter(mailbox => mailbox.status === 'active').map(mailbox => <option key={mailbox.id} value={mailbox.email}>{mailbox.email}</option>)}
                  </select>
                </label>
                <label>Forward to<input value={forwarderForm.to} onChange={event => setForwarderForm(current => ({ ...current, to: event.target.value }))} placeholder="someone@example.com" aria-label="Forwarder target" /></label>
                <div className="row-actions">
                  <button type="submit" className="primary-button" disabled={!forwarderForm.from}><RefreshCw size={15} />Add forwarder</button>
                </div>
              </form>
            </>
          )}

          {tab === 'domain' && (
            <>
              <div className="settings-section">
                <h3>{domain.domain}</h3>
                <div className="billing-plan">
                  <div>
                    <strong>Domain status</strong>
                    <small>{dns.mx && dns.spf && dns.dkim && dns.dmarc ? 'All records verified — mail is flowing.' : `${Object.values(dns).filter(Boolean).length} of ${dnsRecords.length} records verified`}</small>
                  </div>
                  <button type="button" className="secondary-button" onClick={verifyRecords}><RefreshCw size={14} />Check DNS records</button>
                </div>
                {dns.mx && dns.spf && dns.dkim && dns.dmarc && notice.includes('verified') && <p className="settings-notice settings-notice--ok">{notice}</p>}
              </div>
              <div className="settings-section">
                <h3>DNS records</h3>
                {dnsRecords.map(record => (
                  <div className="billing-row" key={record.id}>
                    <div><strong>{record.name}</strong><small className="admin-record">{record.value}</small></div>
                    <span className={`billing-paid ${dns[record.id] ? '' : 'admin-record--pending'}`}>{dns[record.id] && <Check size={13} />}{dns[record.id] ? 'Verified' : 'Required'}</span>
                    <button type="button" className="secondary-button" onClick={() => void copyRecord(record)}>{copied === record.id ? <Check size={14} /> : <Copy size={14} />}{copied === record.id ? 'Copied' : 'Copy'}</button>
                  </div>
                ))}
              </div>
              <div className="settings-section">
                <h3>Catch-all</h3>
                <label className="settings-options"><input type="checkbox" checked={domain.catchAllEnabled} onChange={event => setCatchAllEnabled(event.target.checked)} /> Deliver mail for unknown addresses at {domain.domain}</label>
                {domain.catchAllEnabled && (
                  <label>Deliver to
                    <select value={domain.catchAll} onChange={event => setCatchAllTarget(event.target.value)} aria-label="Catch-all recipient">
                      {mailboxes.filter(mailbox => mailbox.status === 'active').map(mailbox => <option key={mailbox.id} value={mailbox.email}>{mailbox.email}</option>)}
                    </select>
                  </label>
                )}
              </div>
            </>
          )}

          <footer>
            <button type="button" className="primary-button" onClick={close}>Done</button>
          </footer>
        </div>
      </section>
    </div>
  )
}