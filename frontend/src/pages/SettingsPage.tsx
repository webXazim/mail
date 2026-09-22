import { useCallback, useEffect, useState, type FormEvent } from 'react'
import { Link, useNavigate, useSearchParams } from 'react-router-dom'
import {
  ArrowLeft,
  AtSign,
  Bell,
  BellOff,
  Check,
  FileText,
  Filter,
  History as HistoryIcon,
  KeyRound,
  LockKeyhole,
  Mailbox,
  Save,
  ShieldCheck,
  UserRound,
  UsersRound,
} from 'lucide-react'
import { settingsApi, type UserSettings } from '../services/settings'
import { primaryAccount } from '../services/accounts'
import type { RealtimeEvent } from '../services/ws'
import { AccountsSettings } from '../components/settings/AccountsSettings'
import { FiltersSettings } from '../components/settings/FiltersSettings'
import { IdentitiesSettings } from '../components/settings/IdentitiesSettings'
import { MailSettings } from '../components/settings/MailSettings'
import { MailClientsSettings } from '../components/settings/MailClientsSettings'
import { SpamSettings } from '../components/settings/SpamSettings'
import {
  securityApi,
  type AccountSession,
  type TwoFactorSetup,
  type TwoFactorStatus,
} from '../services/security'

type Tab =
  | 'account'
  | 'accounts'
  | 'clients'
  | 'mail'
  | 'filters'
  | 'spam'
  | 'identities'
  | 'notifications'
  | 'security'
  | 'legal'

const tabs: { id: Tab; label: string; icon: typeof UserRound }[] = [
  { id: 'account', label: 'Account', icon: UserRound },
  { id: 'accounts', label: 'Accounts', icon: UsersRound },
  { id: 'clients', label: 'Mail clients', icon: KeyRound },
  { id: 'mail', label: 'Mail', icon: Mailbox },
  { id: 'filters', label: 'Filters', icon: Filter },
  { id: 'spam', label: 'Spam', icon: ShieldCheck },
  { id: 'identities', label: 'Identities', icon: AtSign },
  { id: 'notifications', label: 'Notifications', icon: Bell },
  { id: 'security', label: 'Security', icon: LockKeyhole },
  { id: 'legal', label: 'Legal', icon: FileText },
]

const notificationSupported = () => typeof window !== 'undefined' && 'Notification' in window

export function SettingsPage() {
  const navigate = useNavigate()
  const [searchParams] = useSearchParams()
  const [tab, setTab] = useState<Tab>(() => {
    const requested = searchParams.get('tab') as Tab | null
    return requested && tabs.some((item) => item.id === requested) ? requested : 'account'
  })
  const [settings, setSettings] = useState<UserSettings>(() => settingsApi.load())
  const [saved, setSaved] = useState(false)
  const [permission, setPermission] = useState<'granted' | 'denied' | 'default' | 'unsupported'>(
    () =>
      notificationSupported()
        ? (Notification.permission as 'granted' | 'denied' | 'default')
        : 'unsupported',
  )
  const [password, setPassword] = useState({ current: '', next: '', confirm: '' })
  const [passwordNotice, setPasswordNotice] = useState('')
  const [endNotice, setEndNotice] = useState('')
  const [sessions, setSessions] = useState<AccountSession[]>([])
  const [sessionsLoading, setSessionsLoading] = useState(true)
  const [sessionsError, setSessionsError] = useState('')
  const [twoFactor, setTwoFactor] = useState<TwoFactorStatus | null>(null)
  const [twoFactorLoading, setTwoFactorLoading] = useState(true)
  const [twoFactorSetup, setTwoFactorSetup] = useState<TwoFactorSetup | null>(null)
  const [twoFactorSetupPassword, setTwoFactorSetupPassword] = useState('')
  const [twoFactorCode, setTwoFactorCode] = useState('')
  const [twoFactorCurrentPassword, setTwoFactorCurrentPassword] = useState('')
  const [twoFactorNotice, setTwoFactorNotice] = useState('')
  const [recoveryCodes, setRecoveryCodes] = useState<string[]>([])

  const back = () => navigate('/mail/inbox')

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') navigate('/mail/inbox')
    }
    window.addEventListener('keydown', handleKeyDown)
    return () => window.removeEventListener('keydown', handleKeyDown)
  }, [navigate])

  useEffect(() => {
    let cancelled = false
    void settingsApi.refresh().then((next) => {
      if (!cancelled) setSettings(next)
    })
    return () => {
      cancelled = true
    }
  }, [])

  useEffect(() => {
    const onRealtime = (incoming: Event) => {
      const detail = (incoming as CustomEvent<RealtimeEvent>).detail
      if (detail?.kind !== 'resource-changed') return
      if (detail.payload.resource !== 'settings' && detail.payload.resource !== 'profile') return
      void settingsApi.refresh().then(setSettings).catch(() => {})
    }
    window.addEventListener('cs-mail-realtime', onRealtime)
    return () => window.removeEventListener('cs-mail-realtime', onRealtime)
  }, [])

  const refreshSessions = useCallback(async () => {
    setSessionsLoading(true)
    setSessionsError('')
    try {
      setSessions(await securityApi.sessions())
    } catch (error) {
      setSessions([])
      setSessionsError(error instanceof Error ? error.message : 'Could not load active sessions')
    } finally {
      setSessionsLoading(false)
    }
  }, [])

  useEffect(() => {
    void refreshSessions()
  }, [refreshSessions])

  const refreshTwoFactor = useCallback(async () => {
    setTwoFactorLoading(true)
    try {
      setTwoFactor(await securityApi.twoFactorStatus())
    } catch (error) {
      setTwoFactorNotice(error instanceof Error ? error.message : 'Could not load two-factor status')
    } finally {
      setTwoFactorLoading(false)
    }
  }, [])

  useEffect(() => {
    void refreshTwoFactor()
  }, [refreshTwoFactor])

  const startTwoFactorSetup = async () => {
    setTwoFactorNotice('')
    setRecoveryCodes([])
    try {
      const setup = await securityApi.startTwoFactorSetup(twoFactorSetupPassword)
      setTwoFactorSetup(setup)
      setTwoFactorCode('')
      setTwoFactorSetupPassword('')
    } catch (error) {
      setTwoFactorNotice(error instanceof Error ? error.message : 'Could not start two-factor setup')
    }
  }

  const confirmTwoFactor = async () => {
    setTwoFactorNotice('')
    try {
      const result = await securityApi.confirmTwoFactor(twoFactorCode)
      setRecoveryCodes(result.recovery_codes)
      setTwoFactorSetup(null)
      setTwoFactorCode('')
      setTwoFactorNotice(result.message)
      await Promise.all([refreshTwoFactor(), refreshSessions()])
    } catch (error) {
      setTwoFactorNotice(error instanceof Error ? error.message : 'Could not verify authentication code')
    }
  }

  const disableTwoFactor = async () => {
    setTwoFactorNotice('')
    try {
      const result = await securityApi.disableTwoFactor(twoFactorCurrentPassword, twoFactorCode)
      setTwoFactorCurrentPassword('')
      setTwoFactorCode('')
      setRecoveryCodes([])
      setTwoFactorNotice(result.message)
      await Promise.all([refreshTwoFactor(), refreshSessions()])
    } catch (error) {
      setTwoFactorNotice(error instanceof Error ? error.message : 'Could not disable two-factor authentication')
    }
  }

  const regenerateRecoveryCodes = async () => {
    setTwoFactorNotice('')
    try {
      const result = await securityApi.regenerateRecoveryCodes(
        twoFactorCurrentPassword,
        twoFactorCode,
      )
      setRecoveryCodes(result.recovery_codes)
      setTwoFactorCurrentPassword('')
      setTwoFactorCode('')
      setTwoFactorNotice('New recovery codes generated. Previous recovery codes no longer work.')
      await refreshTwoFactor()
    } catch (error) {
      setTwoFactorNotice(error instanceof Error ? error.message : 'Could not regenerate recovery codes')
    }
  }

  const update = (patch: Partial<UserSettings>) =>
    setSettings((current) => {
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

  const changePassword = async (event: FormEvent) => {
    event.preventDefault()
    setPasswordNotice('')
    if (password.next.length < 12) {
      setPasswordNotice('New password must be at least 12 characters')
      return
    }
    if (password.next !== password.confirm) {
      setPasswordNotice('New passwords do not match')
      return
    }
    try {
      const result = await securityApi.changePassword(password.current, password.next)
      setPasswordNotice(result.message || 'Password updated')
      setPassword({ current: '', next: '', confirm: '' })
      await refreshSessions()
    } catch (error) {
      setPasswordNotice(error instanceof Error ? error.message : 'Could not update password')
    }
  }

  const permissionLabel =
    permission === 'granted'
      ? 'Allowed'
      : permission === 'denied'
        ? 'Blocked in browser'
        : permission === 'unsupported'
          ? 'Not supported'
          : 'Permission needed'

  return (
    <div className="settings-page" role="region" aria-label="Settings">
      <header className="calendar-head">
        <div>
          <p className="eyebrow">CS Mail</p>
          <h1>Settings</h1>
        </div>
        <div className="calendar-head__actions">
          <button type="button" className="secondary-button" onClick={back}>
            <ArrowLeft size={14} />
            Back to inbox
          </button>
        </div>
      </header>

      <nav className="admin-nav" aria-label="Settings sections">
        {tabs.map((tabItem) => {
          const Icon = tabItem.icon
          return (
            <button
              type="button"
              className={tab === tabItem.id ? 'admin-nav--active' : ''}
              aria-current={tab === tabItem.id ? 'page' : undefined}
              onClick={() => setTab(tabItem.id)}
              key={tabItem.id}
            >
              <Icon size={14} />
              {tabItem.label}
            </button>
          )
        })}
      </nav>

      {tab === 'account' && (
        <form
          onSubmit={(event) => {
            event.preventDefault()
            settingsApi.save(settings)
            setSaved(true)
          }}
        >
          <div className="settings-section">
            <h2>Profile</h2>
            <label>
              Display name
              <input
                value={settings.displayName}
                onChange={(event) => update({ displayName: event.target.value })}
              />
            </label>
            <label>
              Email address
              <input value={primaryAccount().email} readOnly />
            </label>
          </div>
          <div className="settings-section">
            <h2>Signature</h2>
            <textarea
              value={settings.signature}
              onChange={(event) => update({ signature: event.target.value })}
              placeholder={'Alex Morgan\nProduct & Operations\nCS Mail'}
            />
            <small className="settings-hint">Added to the bottom of new messages.</small>
          </div>
          <div className="settings-section settings-options">
            <h2>Appearance</h2>
            <label>
              Theme
              <select
                value={settings.theme}
                onChange={(event) => update({ theme: event.target.value as UserSettings['theme'] })}
              >
                <option value="dark">Dark</option>
                <option value="light">Light</option>
              </select>
            </label>
            <label>
              Density
              <select
                value={settings.density}
                onChange={(event) =>
                  update({ density: event.target.value as UserSettings['density'] })
                }
              >
                <option value="comfortable">Comfortable</option>
                <option value="cozy">Cozy</option>
                <option value="compact">Compact</option>
              </select>
            </label>
            <label>
              Corner radius
              <select
                value={settings.radius}
                onChange={(event) =>
                  update({ radius: event.target.value as UserSettings['radius'] })
                }
              >
                <option value="sharp">Sharp</option>
                <option value="subtle">Subtle</option>
                <option value="rounded">Rounded</option>
              </select>
            </label>
            <div className="settings-accent-row" role="group" aria-label="Accent color">
              <span className="settings-accent-label">Accent</span>
              <div className="settings-accent-swatches">
                {(['lime', 'mint', 'aqua', 'violet', 'coral'] as UserSettings['accent'][]).map(
                  (value) => (
                    <button
                      key={value}
                      type="button"
                      aria-label={value}
                      aria-pressed={settings.accent === value}
                      className={`settings-accent-swatch${settings.accent === value ? ' settings-accent-swatch--active' : ''}`}
                      data-accent-swatch={value}
                      onClick={() => update({ accent: value })}
                    />
                  ),
                )}
              </div>
            </div>
          </div>
          <div className="settings-section settings-options">
            <h2>Preferences</h2>
            <label>
              <input
                type="checkbox"
                checked={settings.conversations}
                onChange={(event) => update({ conversations: event.target.checked })}
              />{' '}
              Group messages into conversations
            </label>
            <label>
              <input
                type="checkbox"
                checked={settings.markReadOnOpen}
                onChange={(event) => update({ markReadOnOpen: event.target.checked })}
              />{' '}
              Mark messages read when opened
            </label>
            <label>
              <input
                type="checkbox"
                checked={settings.sendReadReceipts}
                onChange={(event) => update({ sendReadReceipts: event.target.checked })}
              />{' '}
              Send read receipts by default
            </label>
          </div>
          <footer>
            <button type="button" className="secondary-button" onClick={back}>
              Cancel
            </button>
            <button className="primary-button">
              <Save size={15} />
              {saved ? 'Saved' : 'Save changes'}
            </button>
          </footer>
        </form>
      )}

      {tab === 'accounts' && <AccountsSettings />}

      {tab === 'clients' && <MailClientsSettings />}

      {tab === 'mail' && <MailSettings />}

      {tab === 'filters' && <FiltersSettings />}

      {tab === 'spam' && <SpamSettings />}

      {tab === 'identities' && <IdentitiesSettings />}

      {tab === 'notifications' && (
        <div>
          <div className="settings-section">
            <h2>Delivery</h2>
            <label>
              <input
                type="checkbox"
                checked={settings.desktopNotifications}
                onChange={(event) => update({ desktopNotifications: event.target.checked })}
              />{' '}
              Desktop notifications for new mail
            </label>
            <label>
              <input
                type="checkbox"
                checked={settings.alertSound}
                onChange={(event) => update({ alertSound: event.target.checked })}
              />{' '}
              Play an alert sound
            </label>
            <label>
              <input
                type="checkbox"
                checked={settings.unreadBadge}
                onChange={(event) => update({ unreadBadge: event.target.checked })}
              />{' '}
              Show unread badge on the app icon
            </label>
            <label>
              Digest email
              <select
                value={settings.digest}
                onChange={(event) =>
                  update({ digest: event.target.value as UserSettings['digest'] })
                }
              >
                <option value="daily">Daily summary</option>
                <option value="weekly">Weekly summary</option>
                <option value="never">No digest</option>
              </select>
            </label>
          </div>
          <div className="settings-section">
            <h2>Browser permission</h2>
            <div className="billing-plan">
              <div>
                <strong className={permission === 'granted' ? 'billing-paid' : ''}>
                  {permission === 'granted' && <Check size={13} />}
                  {permissionLabel}
                </strong>
                <small>
                  {notificationSupported()
                    ? 'CS Mail will surface alerts in this browser.'
                    : 'This browser does not support app notifications.'}
                </small>
              </div>
              <button
                type="button"
                className="secondary-button"
                onClick={() => void enableNotifications()}
                disabled={permission === 'granted' || permission === 'unsupported'}
              >
                <Bell size={15} />
                Enable
              </button>
            </div>
            <p className="settings-hint">
              {permission === 'denied'
                ? 'Notifications are blocked at the browser level. Allow this site in your browser settings and reload.'
                : 'You can review alerts at any time from the bell in the top bar.'}
            </p>
          </div>
          <footer>
            <button type="button" className="primary-button" onClick={back}>
              Done
            </button>
          </footer>
        </div>
      )}

      {tab === 'security' && (
        <div>
          <form className="settings-section" onSubmit={changePassword}>
            <h2>Change password</h2>
            <label>
              Current password
              <input
                type="password"
                name="current"
                autoComplete="current-password"
                value={password.current}
                onChange={(event) =>
                  setPassword((current) => ({ ...current, current: event.target.value }))
                }
              />
            </label>
            <label>
              New password
              <input
                type="password"
                name="next"
                autoComplete="new-password"
                value={password.next}
                onChange={(event) =>
                  setPassword((current) => ({ ...current, next: event.target.value }))
                }
              />
            </label>
            <label>
              Confirm new password
              <input
                type="password"
                name="confirm"
                autoComplete="new-password"
                value={password.confirm}
                onChange={(event) =>
                  setPassword((current) => ({ ...current, confirm: event.target.value }))
                }
              />
            </label>
            {passwordNotice && (
              <p
                className={`settings-notice ${passwordNotice.startsWith('Password updated') ? 'settings-notice--ok' : ''}`}
              >
                {passwordNotice}
              </p>
            )}
            <button type="submit" className="primary-button">
              <KeyRound size={15} />
              Update password
            </button>
          </form>
          <div className="settings-section">
            <h2>Two-factor authentication</h2>
            {twoFactorLoading && <p className="settings-hint">Loading two-factor status…</p>}
            {!twoFactorLoading && twoFactor && (
              <div className="billing-plan">
                <div>
                  <strong>{twoFactor.enabled ? 'Enabled' : 'Not enabled'}</strong>
                  <small>
                    {twoFactor.enabled
                      ? `${twoFactor.recovery_codes_remaining} recovery code${twoFactor.recovery_codes_remaining === 1 ? '' : 's'} remaining`
                      : 'Protect sign-in with a TOTP authenticator app'}
                  </small>
                </div>
                {twoFactor.enabled && (
                  <span className="billing-paid">
                    <ShieldCheck size={13} /> Active
                  </span>
                )}
              </div>
            )}

            {!twoFactor?.enabled && !twoFactorSetup && (
              <div className="settings-security-action">
                <label>
                  Current password
                  <input
                    type="password"
                    autoComplete="current-password"
                    value={twoFactorSetupPassword}
                    onChange={(event) => setTwoFactorSetupPassword(event.target.value)}
                    placeholder="Confirm your password"
                  />
                </label>
                <button
                  type="button"
                  className="secondary-button"
                  disabled={!twoFactorSetupPassword}
                  onClick={startTwoFactorSetup}
                >
                  <ShieldCheck size={15} /> Set up authenticator
                </button>
              </div>
            )}

            {twoFactorSetup && (
              <div className="two-factor-setup">
                <p className="settings-hint">
                  Scan this QR code with your authenticator app, then enter the six-digit code to confirm setup.
                </p>
                <img
                  className="two-factor-qr"
                  src={`data:image/svg+xml;charset=utf-8,${encodeURIComponent(twoFactorSetup.qr_svg)}`}
                  alt="QR code for adding CS Mail to an authenticator app"
                />
                <label>
                  Manual setup key
                  <input value={twoFactorSetup.secret} readOnly spellCheck={false} />
                </label>
                <label>
                  Authenticator code
                  <input
                    type="text"
                    inputMode="numeric"
                    autoComplete="one-time-code"
                    value={twoFactorCode}
                    onChange={(event) => setTwoFactorCode(event.target.value)}
                    placeholder="123456"
                  />
                </label>
                <div className="row-actions">
                  <button
                    type="button"
                    className="primary-button"
                    disabled={!twoFactorCode.trim()}
                    onClick={confirmTwoFactor}
                  >
                    Confirm and enable
                  </button>
                  <button
                    type="button"
                    className="secondary-button"
                    onClick={() => {
                      setTwoFactorSetup(null)
                      setTwoFactorCode('')
                    }}
                  >
                    Cancel
                  </button>
                </div>
              </div>
            )}

            {twoFactor?.enabled && (
              <div className="settings-security-action">
                <p className="settings-hint">
                  To disable two-factor authentication or replace your recovery codes, confirm both your password and an authenticator/recovery code.
                </p>
                <label>
                  Current password
                  <input
                    type="password"
                    autoComplete="current-password"
                    value={twoFactorCurrentPassword}
                    onChange={(event) => setTwoFactorCurrentPassword(event.target.value)}
                  />
                </label>
                <label>
                  Authenticator or recovery code
                  <input
                    type="text"
                    autoComplete="one-time-code"
                    value={twoFactorCode}
                    onChange={(event) => setTwoFactorCode(event.target.value)}
                  />
                </label>
                <div className="row-actions">
                  <button
                    type="button"
                    className="secondary-button"
                    disabled={!twoFactorCurrentPassword || !twoFactorCode.trim()}
                    onClick={regenerateRecoveryCodes}
                  >
                    Generate new recovery codes
                  </button>
                  <button
                    type="button"
                    className="secondary-button"
                    disabled={!twoFactorCurrentPassword || !twoFactorCode.trim()}
                    onClick={disableTwoFactor}
                  >
                    Disable two-factor
                  </button>
                </div>
              </div>
            )}

            {recoveryCodes.length > 0 && (
              <div className="two-factor-recovery">
                <strong>Save these recovery codes now</strong>
                <p className="settings-hint">
                  Each code works once. Store them somewhere secure; CS Mail will not show this set again.
                </p>
                <div className="two-factor-recovery-grid">
                  {recoveryCodes.map((code) => (
                    <code key={code}>{code}</code>
                  ))}
                </div>
                <button
                  type="button"
                  className="secondary-button"
                  onClick={() => void navigator.clipboard?.writeText(recoveryCodes.join('\n'))}
                >
                  Copy recovery codes
                </button>
              </div>
            )}
            {twoFactorNotice && <p className="settings-notice">{twoFactorNotice}</p>}
          </div>
          <div className="settings-section settings-options">
            <h2>Link safety</h2>
            <label>
              <input
                type="checkbox"
                checked={settings.safeLinks}
                onChange={(event) => update({ safeLinks: event.target.checked })}
              />{' '}
              Ask for confirmation before opening unknown links
            </label>
          </div>
          <div className="settings-section">
            <h2>Active sessions</h2>
            {sessionsLoading && <p className="settings-hint">Loading sessions…</p>}
            {!sessionsLoading && sessionsError && (
              <p className="settings-hint">{sessionsError}</p>
            )}
            {!sessionsLoading && !sessionsError && sessions.length === 0 && (
              <p className="settings-hint">No active sessions found.</p>
            )}
            {sessions.map((session) => (
              <div className="billing-row" key={session.id}>
                <div>
                  <strong>{session.device}</strong>
                  <small>
                    {session.ip || 'Unknown IP'} · Last active{' '}
                    {new Date(session.last_used_at).toLocaleString()}
                  </small>
                </div>
                {session.current ? (
                  <span className="billing-paid">
                    <ShieldCheck size={13} />
                    Current
                  </span>
                ) : (
                  <button
                    type="button"
                    className="secondary-button"
                    onClick={async () => {
                      try {
                        await securityApi.revokeSession(session.id)
                        setEndNotice('Session signed out')
                        await refreshSessions()
                      } catch (error) {
                        setEndNotice(error instanceof Error ? error.message : 'Could not sign out session')
                      }
                    }}
                  >
                    Sign out
                  </button>
                )}
              </div>
            ))}
            <p className="settings-hint">
              Sign out every other browser or device while keeping this session active.
            </p>
            <div className="row-actions">
              <button
                type="button"
                className="secondary-button"
                onClick={async () => {
                  try {
                    const result = await securityApi.revokeOtherSessions()
                    setEndNotice(
                      result.revoked > 0
                        ? `Signed out ${result.revoked} other session${result.revoked === 1 ? '' : 's'}`
                        : 'No other active sessions',
                    )
                    await refreshSessions()
                  } catch (error) {
                    setEndNotice(error instanceof Error ? error.message : 'Could not sign out sessions')
                  }
                }}
              >
                <BellOff size={15} />
                Sign out other sessions
              </button>
              {endNotice && <small className="settings-notice settings-notice--ok">{endNotice}</small>}
            </div>
          </div>
          <div className="settings-section">
            <div className="billing-plan">
              <div>
                <strong>Account activity</strong>
                <small>Sign-ins, security and billing events for your account</small>
              </div>
              <button
                type="button"
                className="secondary-button"
                onClick={() => navigate('/mail/audit-log')}
              >
                <HistoryIcon size={14} />
                Open audit log
              </button>
            </div>
          </div>
          <div className="settings-section settings-options">
            <h2>Legal</h2>
            <label>
              <Link to="/legal/terms">Terms of Service</Link>
            </label>
            <label>
              <Link to="/legal/privacy">Privacy Policy</Link>
            </label>
            <label>
              <Link to="/legal/aup">Acceptable Use Policy</Link>
            </label>
            <label>
              <Link to="/legal/security">Security &amp; Compliance</Link>
            </label>
          </div>
          <footer>
            <button type="button" className="primary-button" onClick={back}>
              Done
            </button>
          </footer>
        </div>
      )}

      {tab === 'legal' && (
        <div>
          <div className="settings-section settings-options">
            <h2>Legal</h2>
            <label>
              <Link to="/legal/terms">Terms of Service</Link>
            </label>
            <label>
              <Link to="/legal/privacy">Privacy Policy</Link>
            </label>
            <label>
              <Link to="/legal/aup">Acceptable Use Policy</Link>
            </label>
            <label>
              <Link to="/legal/security">Security &amp; Compliance</Link>
            </label>
          </div>
          <footer>
            <button type="button" className="primary-button" onClick={back}>
              Done
            </button>
          </footer>
        </div>
      )}
    </div>
  )
}
