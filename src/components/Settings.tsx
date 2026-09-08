import { useEffect, useRef, useState, type FormEvent } from 'react'
import { AtSign, Bell, BellOff, Check, Filter, KeyRound, LockKeyhole, Mailbox, Save, ShieldCheck, UserRound, X } from 'lucide-react'
import { useFocusTrap } from '../hooks/useFocusTrap'
import { settingsApi, type UserSettings } from '../services/settings'
import { FiltersSettings } from './settings/FiltersSettings'
import { IdentitiesSettings } from './settings/IdentitiesSettings'
import { MailSettings } from './settings/MailSettings'
import { SpamSettings } from './settings/SpamSettings'

export type SettingsProps = { close: () => void }

type Tab = 'account' | 'mail' | 'filters' | 'spam' | 'identities' | 'notifications' | 'security'

const sessions = [
  { id: 1, name: 'Chrome on macOS', location: 'San Francisco, US', active: true },
  { id: 2, name: 'iOS Mail', location: 'San Francisco, US', active: false },
  { id: 3, name: 'Firefox on Windows', location: 'Berlin, DE', active: false },
]

const notificationSupported = () => typeof window !== 'undefined' && 'Notification' in window

export function Settings({ close }: SettingsProps) {
  const [settings, setSettings] = useState<UserSettings>(() => settingsApi.load())
  const [tab, setTab] = useState<Tab>('account')
  const [saved, setSaved] = useState(false)
  const panelRef = useRef<HTMLElement>(null)
  useFocusTrap(panelRef)
  const [permission, setPermission] = useState<'granted' | 'denied' | 'default' | 'unsupported'>(() =>
    notificationSupported() ? (Notification.permission as 'granted' | 'denied' | 'default') : 'unsupported',
  )
  const [password, setPassword] = useState({ current: '', next: '', confirm: '' })
  const [passwordNotice, setPasswordNotice] = useState('')
  const [endNotice, setEndNotice] = useState('')

  const update = (patch: Partial<UserSettings>) => setSettings(current => {
    const next = { ...current, ...patch }
    settingsApi.save(next)
    return next
  })
  useEffect(() => {
    if (!saved) return
    const timer = window.setTimeout(() => setSaved(false), 1800)
    return () => window.clearTimeout(timer)
  }, [saved])

  const enableNotifications = async () => {
    if (!notificationSupported()) return
    const result = await Notification.requestPermission()
    setPermission(result as 'granted' | 'denied' | 'default')
    if (result === 'granted') update({ desktopNotifications: true })
  }

  const changePassword = (event: FormEvent) => {
    event.preventDefault()
    if (password.current !== 'current-password') { setPasswordNotice('Current password is incorrect'); return }
    if (password.next.length < 8) { setPasswordNotice('New password must be at least 8 characters'); return }
    if (password.next !== password.confirm) { setPasswordNotice('New passwords do not match'); return }
    setPasswordNotice('Password updated')
    setPassword({ current: '', next: '', confirm: '' })
  }

  const permissionLabel = permission === 'granted' ? 'Allowed' : permission === 'denied' ? 'Blocked in browser' : permission === 'unsupported' ? 'Not supported' : 'Permission needed'

  return (
    <div className="settings-layer" role="presentation">
      <section ref={panelRef} className="settings-panel" role="dialog" aria-modal="true" aria-labelledby="settings-title">
        <header>
          <div><p className="eyebrow">Harbor Mail</p><h2 id="settings-title">Settings</h2></div>
          <button type="button" className="icon-button" aria-label="Close settings" onClick={close}><X size={17} /></button>
        </header>
        <nav className="settings-nav" aria-label="Settings sections">
          <button type="button" className={tab === 'account' ? 'settings-nav--active' : ''} aria-current={tab === 'account' ? 'page' : undefined} onClick={() => setTab('account')}><UserRound size={15} />Account</button>
          <button type="button" className={tab === 'mail' ? 'settings-nav--active' : ''} aria-current={tab === 'mail' ? 'page' : undefined} onClick={() => setTab('mail')}><Mailbox size={15} />Mail</button>
          <button type="button" className={tab === 'filters' ? 'settings-nav--active' : ''} aria-current={tab === 'filters' ? 'page' : undefined} onClick={() => setTab('filters')}><Filter size={15} />Filters</button>
          <button type="button" className={tab === 'spam' ? 'settings-nav--active' : ''} aria-current={tab === 'spam' ? 'page' : undefined} onClick={() => setTab('spam')}><ShieldCheck size={15} />Spam</button>
          <button type="button" className={tab === 'identities' ? 'settings-nav--active' : ''} aria-current={tab === 'identities' ? 'page' : undefined} onClick={() => setTab('identities')}><AtSign size={15} />Identities</button>
          <button type="button" className={tab === 'notifications' ? 'settings-nav--active' : ''} aria-current={tab === 'notifications' ? 'page' : undefined} onClick={() => setTab('notifications')}><Bell size={15} />Notifications</button>
          <button type="button" className={tab === 'security' ? 'settings-nav--active' : ''} aria-current={tab === 'security' ? 'page' : undefined} onClick={() => setTab('security')}><LockKeyhole size={15} />Security</button>
        </nav>

        {tab === 'account' && (
          <form onSubmit={event => { event.preventDefault(); settingsApi.save(settings); setSaved(true) }}>
            <div className="settings-section">
              <h3>Profile</h3>
              <label>Display name<input value={settings.displayName} onChange={event => update({ displayName: event.target.value })} /></label>
              <label>Email address<input value="alex@harbor.co" readOnly /></label>
            </div>
            <div className="settings-section">
              <h3>Signature</h3>
              <textarea value={settings.signature} onChange={event => update({ signature: event.target.value })} placeholder={'Alex Morgan\nProduct & Operations\nHarbor Mail'} />
              <small className="settings-hint">Added to the bottom of new messages.</small>
            </div>
            <div className="settings-section settings-options">
              <h3>Preferences</h3>
              <label><input type="checkbox" checked={settings.conversations} onChange={event => update({ conversations: event.target.checked })} /> Group messages into conversations</label>
              <label><input type="checkbox" checked={settings.markReadOnOpen} onChange={event => update({ markReadOnOpen: event.target.checked })} /> Mark messages read when opened</label>
              <label><input type="checkbox" checked={settings.sendReadReceipts} onChange={event => update({ sendReadReceipts: event.target.checked })} /> Send read receipts by default</label>
            </div>
            <footer>
              <button type="button" className="secondary-button" onClick={close}>Cancel</button>
              <button className="primary-button"><Save size={15} />{saved ? 'Saved' : 'Save changes'}</button>
            </footer>
          </form>
        )}

        {tab === 'mail' && <MailSettings />}

        {tab === 'filters' && <FiltersSettings />}

        {tab === 'spam' && <SpamSettings />}

        {tab === 'identities' && <IdentitiesSettings />}

        {tab === 'notifications' && (
          <div>
            <div className="settings-section">
              <h3>Delivery</h3>
              <label><input type="checkbox" checked={settings.desktopNotifications} onChange={event => update({ desktopNotifications: event.target.checked })} /> Desktop notifications for new mail</label>
              <label><input type="checkbox" checked={settings.alertSound} onChange={event => update({ alertSound: event.target.checked })} /> Play an alert sound</label>
              <label><input type="checkbox" checked={settings.unreadBadge} onChange={event => update({ unreadBadge: event.target.checked })} /> Show unread badge on the app icon</label>
              <label>Digest email
                <select value={settings.digest} onChange={event => update({ digest: event.target.value as UserSettings['digest'] })}>
                  <option value="daily">Daily summary</option>
                  <option value="weekly">Weekly summary</option>
                  <option value="never">No digest</option>
                </select>
              </label>
            </div>
            <div className="settings-section">
              <h3>Browser permission</h3>
              <div className="billing-plan">
                <div>
                  <strong className={permission === 'granted' ? 'billing-paid' : ''}>{permission === 'granted' && <Check size={13} />}{permissionLabel}</strong>
                  <small>{notificationSupported() ? 'Harbor Mail will surface alerts in this browser.' : 'This browser does not support app notifications.'}</small>
                </div>
                <button type="button" className="secondary-button" onClick={() => void enableNotifications()} disabled={permission === 'granted' || permission === 'unsupported'}><Bell size={15} />Enable</button>
              </div>
              <p className="settings-hint">{permission === 'denied' ? 'Notifications are blocked at the browser level. Allow this site in your browser settings and reload.' : 'You can review alerts at any time from the bell in the top bar.'}</p>
            </div>
            <footer>
              <button type="button" className="primary-button" onClick={close}>Done</button>
            </footer>
          </div>
        )}

        {tab === 'security' && (
          <div>
            <form className="settings-section" onSubmit={changePassword}>
              <h3>Change password</h3>
              <label>Current password<input type="password" name="current" autoComplete="current-password" value={password.current} onChange={event => setPassword(current => ({ ...current, current: event.target.value }))} /></label>
              <label>New password<input type="password" name="next" autoComplete="new-password" value={password.next} onChange={event => setPassword(current => ({ ...current, next: event.target.value }))} /></label>
              <label>Confirm new password<input type="password" name="confirm" autoComplete="new-password" value={password.confirm} onChange={event => setPassword(current => ({ ...current, confirm: event.target.value }))} /></label>
              {passwordNotice && <p className={`settings-notice ${passwordNotice === 'Password updated' ? 'settings-notice--ok' : ''}`}>{passwordNotice}</p>}
              <button type="submit" className="primary-button"><KeyRound size={15} />Update password</button>
            </form>
            <div className="settings-section settings-options">
              <h3>Sign-in & verification</h3>
              <label><input type="checkbox" checked={settings.twoFactor} onChange={event => update({ twoFactor: event.target.checked })} /> Require two-factor authentication</label>
              <label><input type="checkbox" checked={settings.safeLinks} onChange={event => update({ safeLinks: event.target.checked })} /> Ask for confirmation before opening unknown links</label>
            </div>
            <div className="settings-section">
              <h3>Active sessions</h3>
              {sessions.map(session => (
                <div className="billing-row" key={session.id}>
                  <div><strong>{session.name}</strong><small>{session.location}{session.active ? '' : ' · Signed out'}</small></div>
                  <span className={session.active ? 'billing-paid' : ''}>{session.active && <><ShieldCheck size={13} />Active</>}</span>
                </div>
              ))}
              <p className="settings-hint">Sign out of this and every other session? This clears Harbor Mail from those devices.</p>
              <div className="row-actions">
                <button type="button" className="secondary-button" onClick={() => setEndNotice('Signed out of other sessions')}><BellOff size={15} />Sign out other sessions</button>
                {endNotice && <small className="settings-notice settings-notice--ok">{endNotice}</small>}
              </div>
            </div>
            <footer>
              <button type="button" className="primary-button" onClick={close}>Done</button>
            </footer>
          </div>
        )}
      </section>
    </div>
  )
}