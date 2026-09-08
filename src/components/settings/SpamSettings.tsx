import { useState } from 'react'
import { Plus, ShieldOff, Trash2 } from 'lucide-react'
import { spamApi, type SpamLevel } from '../../services/spam'

const levelDescriptions: Record<SpamLevel, string> = {
  low: 'Fewer messages are treated as junk. You may see more spam.',
  medium: 'Balanced protection against unwanted mail.',
  aggressive: 'More messages are moved to spam. Check spam regularly.',
}

export function SpamSettings() {
  const [settings, setSettings] = useState(() => spamApi.load())
  const [blockedInput, setBlockedInput] = useState('')
  const [allowedInput, setAllowedInput] = useState('')
  const [notice, setNotice] = useState('')

  const matchingEmail = (value: string) => /^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(value.trim())

  const block = () => {
    if (!matchingEmail(blockedInput)) { setNotice('Enter a valid email address to block.'); return }
    setSettings(spamApi.block(blockedInput))
    setBlockedInput('')
    setNotice('Sender blocked')
  }
  const allow = () => {
    if (!matchingEmail(allowedInput)) { setNotice('Enter a valid email address to allow.'); return }
    setSettings(spamApi.allow(allowedInput))
    setAllowedInput('')
    setNotice('Sender added to the allow list')
  }

  return (
    <div>
      <div className="settings-section">
        <h3>Junk filtering</h3>
        <label>Protection level
          <select value={settings.spamLevel} aria-label="Junk filter level" onChange={event => setSettings(spamApi.save({ ...settings, spamLevel: event.target.value as SpamLevel }))}>
            <option value="low">Low</option>
            <option value="medium">Medium</option>
            <option value="aggressive">Aggressive</option>
          </select>
        </label>
        <p className="settings-hint">{levelDescriptions[settings.spamLevel]}</p>
        <p className="settings-hint">Harbor Mail learns from links you mark as spam or not-spam in the mailbox.</p>
      </div>

      <div className="settings-section">
        <h3>Blocked senders</h3>
        <p className="settings-hint">Messages from these addresses are always treated as junk.</p>
        <div className="spam-add">
          <input value={blockedInput} aria-label="Block an address" placeholder="sender@example.com" onChange={event => setBlockedInput(event.target.value)} onKeyDown={event => { if (event.key === 'Enter') { event.preventDefault(); block() } }} />
          <button type="button" className="primary-button" onClick={block}><ShieldOff size={15} />Block</button>
        </div>
        {settings.blocked.map(address => (
          <div className="billing-row" key={address}>
            <div><strong>{address}</strong><small>Blocked sender</small></div>
            <button type="button" className="icon-button" aria-label={`Unblock ${address}`} onClick={() => setSettings(spamApi.removeBlocked(address))}><Trash2 size={15} /></button>
          </div>
        ))}
        {settings.blocked.length === 0 && <p className="settings-hint">No blocked senders yet.</p>}
      </div>

      <div className="settings-section">
        <h3>Allowed senders</h3>
        <p className="settings-hint">Messages from these addresses are never treated as junk.</p>
        <div className="spam-add">
          <input value={allowedInput} aria-label="Allow an address" placeholder="friend@example.com" onChange={event => setAllowedInput(event.target.value)} onKeyDown={event => { if (event.key === 'Enter') { event.preventDefault(); allow() } }} />
          <button type="button" className="primary-button" onClick={allow}><Plus size={15} />Allow</button>
        </div>
        {settings.allowed.map(address => (
          <div className="billing-row" key={address}>
            <div><strong>{address}</strong><small>Allowed sender</small></div>
            <button type="button" className="icon-button" aria-label={`Remove ${address} from allow list`} onClick={() => setSettings(spamApi.removeAllowed(address))}><Trash2 size={15} /></button>
          </div>
        ))}
        {settings.allowed.length === 0 && <p className="settings-hint">No allowed senders yet.</p>}
      </div>

      {notice && <p className="settings-notice settings-notice--ok">{notice}</p>}
    </div>
  )
}