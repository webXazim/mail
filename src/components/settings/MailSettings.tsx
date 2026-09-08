import { useEffect, useState } from 'react'
import { Save } from 'lucide-react'
import { forwardingApi } from '../../services/forwarding'
import { vacationApi } from '../../services/vacation'

export function MailSettings() {
  const [vacation, setVacation] = useState(() => vacationApi.load())
  const [forwarding, setForwarding] = useState(() => forwardingApi.load())
  const [saved, setSaved] = useState(false)

  useEffect(() => {
    if (!saved) return
    const timer = window.setTimeout(() => setSaved(false), 1800)
    return () => window.clearTimeout(timer)
  }, [saved])

  const updateVacation = (patch: Partial<typeof vacation>) => setVacation(current => vacationApi.save({ ...current, ...patch }) as typeof current)
  const updateForwarding = (patch: Partial<typeof forwarding>) => setForwarding(current => forwardingApi.save({ ...current, ...patch }) as typeof current)

  const dateValue = (value: string) => value || ''

  return (
    <div>
      <div className="settings-section">
        <h3>Auto-reply (vacation responder)</h3>
        <p className="settings-hint">Send an automatic reply to people who write to you while you are away.</p>
        <label className="settings-options"><input type="checkbox" checked={vacation.enabled} onChange={event => updateVacation({ enabled: event.target.checked })} /> Turn on automatic replies</label>
        {vacation.enabled && (
          <>
            <label>Subject<input value={vacation.subject} aria-label="Auto-reply subject" onChange={event => updateVacation({ subject: event.target.value })} /></label>
            <label>Message<textarea value={vacation.message} aria-label="Auto-reply message" onChange={event => updateVacation({ message: event.target.value })} /></label>
            <div className="vacation-range">
              <label>Replies start<input type="date" value={dateValue(vacation.startsAt)} onChange={event => updateVacation({ startsAt: event.target.value })} /></label>
              <label>Replies end<input type="date" value={dateValue(vacation.endsAt)} onChange={event => updateVacation({ endsAt: event.target.value })} /></label>
            </div>
            <label className="settings-options"><input type="checkbox" checked={vacation.onlyContacts} onChange={event => updateVacation({ onlyContacts: event.target.checked })} /> Only reply to people in my contacts</label>
          </>
        )}
      </div>

      <div className="settings-section">
        <h3>Forwarding</h3>
        <p className="settings-hint">Forward incoming mail to another address.</p>
        <label className="settings-options"><input type="checkbox" checked={forwarding.enabled} onChange={event => updateForwarding({ enabled: event.target.checked })} /> Forward incoming messages</label>
        {forwarding.enabled && (
          <>
            <label>Forward to<input type="email" value={forwarding.address} aria-label="Forwarding address" placeholder="other@example.com" onChange={event => updateForwarding({ address: event.target.value })} /></label>
            <label className="settings-options"><input type="checkbox" checked={forwarding.keepCopy} onChange={event => updateForwarding({ keepCopy: event.target.checked })} /> Keep a copy in my mailbox</label>
            {!forwarding.address.includes('@') && <p className="settings-hint">Add the address you want mail forwarded to.</p>}
          </>
        )}
      </div>

      <footer>
        <button type="button" className="primary-button" onClick={() => setSaved(true)}><Save size={15} />{saved ? 'Saved' : 'Save changes'}</button>
      </footer>
    </div>
  )
}