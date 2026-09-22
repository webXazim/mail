import { useEffect, useState } from 'react'
import { Save } from 'lucide-react'
import { forwardingApi, type ForwardingSettings } from '../../services/forwarding'
import { vacationApi, type VacationSettings } from '../../services/vacation'
import type { RealtimeEvent } from '../../services/ws'

export function MailSettings() {
  const [vacation, setVacation] = useState<VacationSettings>(() => vacationApi.load())
  const [forwarding, setForwarding] = useState<ForwardingSettings>(() => forwardingApi.load())
  const [verificationCode, setVerificationCode] = useState('')
  const [notice, setNotice] = useState('')
  const [error, setError] = useState('')
  const [saving, setSaving] = useState(false)
  const [loading, setLoading] = useState(true)

  useEffect(() => {
    let active = true
    Promise.all([forwardingApi.refresh(), vacationApi.refresh()])
      .then(([forwardResult, vacationResult]) => {
        if (!active) return
        setForwarding(forwardResult.forwarding)
        setVacation(vacationResult.vacation)
      })
      .catch((reason: unknown) => {
        if (active) setError(reason instanceof Error ? reason.message : 'Could not load mail settings')
      })
      .finally(() => active && setLoading(false))
    return () => {
      active = false
    }
  }, [])

  useEffect(() => {
    const onRealtime = (incoming: Event) => {
      const detail = (incoming as CustomEvent<RealtimeEvent>).detail
      if (detail?.kind !== 'resource-changed' || detail.payload.resource !== 'automation') return
      void Promise.all([forwardingApi.refresh(), vacationApi.refresh()])
        .then(([forwardResult, vacationResult]) => {
          setForwarding(forwardResult.forwarding)
          setVacation(vacationResult.vacation)
        })
        .catch(() => {})
    }
    window.addEventListener('cs-mail-realtime', onRealtime)
    return () => window.removeEventListener('cs-mail-realtime', onRealtime)
  }, [])

  useEffect(() => {
    if (!notice) return
    const timer = window.setTimeout(() => setNotice(''), 2600)
    return () => window.clearTimeout(timer)
  }, [notice])

  const dateValue = (value: string) => value || ''

  const save = async () => {
    setSaving(true)
    setError('')
    try {
      const [forwardResult, vacationResult] = await Promise.all([
        forwardingApi.save(forwarding),
        vacationApi.save(vacation),
      ])
      setForwarding(forwardResult.forwarding)
      setVacation(vacationResult.vacation)
      if (forwardResult.forwarding.verificationPending) {
        setNotice('Verification code sent to the forwarding address')
      } else if (!forwardResult.sync.inSync || !vacationResult.sync.inSync) {
        setNotice('Saved — mail-server sync is queued')
      } else {
        setNotice('Mail settings saved')
      }
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : 'Could not save mail settings')
    } finally {
      setSaving(false)
    }
  }

  const verifyForwarding = async () => {
    if (!verificationCode.trim()) return
    setSaving(true)
    setError('')
    try {
      const result = await forwardingApi.verify(verificationCode.trim())
      setForwarding(result.forwarding)
      setVerificationCode('')
      setNotice(result.sync.inSync ? 'Forwarding verified and enabled' : 'Verified — sync is queued')
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : 'Could not verify forwarding')
    } finally {
      setSaving(false)
    }
  }

  const resendVerification = async () => {
    setSaving(true)
    setError('')
    try {
      await forwardingApi.resend()
      setNotice('A new verification code was sent')
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : 'Could not resend the verification code')
    } finally {
      setSaving(false)
    }
  }

  return (
    <div aria-busy={loading || saving}>
      <div className="settings-section">
        <h3>Auto-reply (vacation responder)</h3>
        <p className="settings-hint">
          Automatic replies are processed by your mailbox even when this browser is closed.
        </p>
        <label className="settings-options">
          <input
            type="checkbox"
            checked={vacation.enabled}
            onChange={(event) => setVacation((current) => ({ ...current, enabled: event.target.checked }))}
          />{' '}
          Turn on automatic replies
        </label>
        {vacation.enabled && (
          <>
            <label>
              Subject
              <input
                value={vacation.subject}
                aria-label="Auto-reply subject"
                onChange={(event) => setVacation((current) => ({ ...current, subject: event.target.value }))}
              />
            </label>
            <label>
              Message
              <textarea
                value={vacation.message}
                aria-label="Auto-reply message"
                onChange={(event) => setVacation((current) => ({ ...current, message: event.target.value }))}
              />
            </label>
            <div className="vacation-range">
              <label>
                Replies start
                <input
                  type="date"
                  value={dateValue(vacation.startsAt)}
                  onChange={(event) => setVacation((current) => ({ ...current, startsAt: event.target.value }))}
                />
              </label>
              <label>
                Replies end
                <input
                  type="date"
                  value={dateValue(vacation.endsAt)}
                  onChange={(event) => setVacation((current) => ({ ...current, endsAt: event.target.value }))}
                />
              </label>
            </div>
            <label className="settings-options">
              <input
                type="checkbox"
                checked={vacation.onlyContacts}
                onChange={(event) => setVacation((current) => ({ ...current, onlyContacts: event.target.checked }))}
              />{' '}
              Only reply to people in my contacts
            </label>
          </>
        )}
      </div>

      <div className="settings-section">
        <h3>Forwarding</h3>
        <p className="settings-hint">
          External destinations must be verified before the mailbox can forward incoming mail.
        </p>
        <label className="settings-options">
          <input
            type="checkbox"
            checked={forwarding.enabled}
            onChange={(event) => setForwarding((current) => ({ ...current, enabled: event.target.checked }))}
          />{' '}
          Forward incoming messages
        </label>
        {(forwarding.enabled || forwarding.verificationPending) && (
          <>
            <label>
              Forward to
              <input
                type="email"
                value={forwarding.address}
                aria-label="Forwarding address"
                placeholder="other@example.com"
                onChange={(event) =>
                  setForwarding((current) => ({
                    ...current,
                    address: event.target.value,
                    verified: false,
                    verificationPending: false,
                  }))
                }
              />
            </label>
            <label className="settings-options">
              <input
                type="checkbox"
                checked={forwarding.keepCopy}
                onChange={(event) => setForwarding((current) => ({ ...current, keepCopy: event.target.checked }))}
              />{' '}
              Keep a copy in my mailbox
            </label>
          </>
        )}

        {forwarding.verificationPending && (
          <div className="rule-card">
            <strong>Verify forwarding address</strong>
            <p className="settings-hint">
              Enter the code sent to <strong>{forwarding.address}</strong>. Forwarding stays off until verification succeeds.
            </p>
            <label>
              Verification code
              <input
                value={verificationCode}
                autoComplete="one-time-code"
                aria-label="Forwarding verification code"
                onChange={(event) => setVerificationCode(event.target.value.toUpperCase())}
              />
            </label>
            <div className="row-actions">
              <button type="button" className="secondary-button" onClick={resendVerification} disabled={saving}>
                Resend code
              </button>
              <button type="button" className="primary-button" onClick={verifyForwarding} disabled={saving || !verificationCode.trim()}>
                Verify and enable
              </button>
            </div>
          </div>
        )}

        {forwarding.verified && forwarding.address && (
          <p className="settings-notice settings-notice--ok">Verified destination: {forwarding.address}</p>
        )}
      </div>

      {error && <p className="settings-notice">{error}</p>}
      {notice && <p className="settings-notice settings-notice--ok">{notice}</p>}

      <footer>
        <button type="button" className="primary-button" onClick={save} disabled={saving || loading}>
          <Save size={15} />
          {saving ? 'Saving…' : 'Save changes'}
        </button>
      </footer>
    </div>
  )
}
