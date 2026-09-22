import { useEffect, useRef, useState, type FormEvent } from 'react'
import { Bell, Inbox, Keyboard, Zap } from 'lucide-react'
import { useFocusTrap } from '../hooks/useFocusTrap'
import { isRemoteMail } from '../services/remote-mail'
import { profileApi } from '../services/profile'

const onboardedKey = 'cs-mail:onboarded'

type Step = 'hidden' | 'checking' | 'welcome' | 'setup'

const points = [
  { icon: Inbox, text: 'Conversations grouped, priorities surfaced' },
  { icon: Zap, text: 'Send on your schedule — queue, snooze, undo' },
  { icon: Keyboard, text: 'Drive the whole client from the keyboard' },
  { icon: Bell, text: 'Notifications that respect your focus' },
]

export function Onboarding() {
  const remote = isRemoteMail()
  const [step, setStep] = useState<Step>(() => {
    if (remote) return 'checking'
    return localStorage.getItem(onboardedKey) === '1' ? 'hidden' : 'welcome'
  })
  const [name, setName] = useState('')
  const [error, setError] = useState('')
  const [saving, setSaving] = useState(false)
  const cardRef = useRef<HTMLElement>(null)
  useFocusTrap(cardRef, step === 'welcome' || step === 'setup', step === 'setup' ? 'input' : '')

  // Real accounts are bootstrapped from the server: no welcome nag once the
  // user has picked a display name (Stalwart mailbox is already provisioned).
  useEffect(() => {
    if (!remote) return
    let cancelled = false
    void profileApi.refresh().then((profile) => {
      if (cancelled) return
      if (profile && !profile.onboarded) {
        setName(profile.display_name.trim() || profile.email.split('@')[0])
        setStep('setup')
      } else {
        setStep('hidden')
      }
    })
    return () => {
      cancelled = true
    }
  }, [remote])

  const dismiss = () => {
    localStorage.setItem(onboardedKey, '1')
    setStep('hidden')
  }

  const finishSetup = async (event: FormEvent) => {
    event.preventDefault()
    const display = name.trim()
    if (!display) {
      setError('Enter a name to continue')
      return
    }
    setSaving(true)
    setError('')
    try {
      await profileApi.update({ display_name: display, onboarded: true })
      localStorage.setItem(onboardedKey, '1')
      setStep('hidden')
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : 'Unable to save your name')
      setSaving(false)
    }
  }

  useEffect(() => {
    if (step === 'hidden' || step === 'checking') return
    const esc = (event: KeyboardEvent) => {
      if (event.key === 'Escape' && step === 'welcome') {
        localStorage.setItem(onboardedKey, '1')
        setStep('hidden')
      }
    }
    window.addEventListener('keydown', esc)
    return () => window.removeEventListener('keydown', esc)
  }, [step])

  if (step === 'hidden' || step === 'checking') return null

  return (
    <div className="onboard-layer">
      <section
        ref={cardRef}
        className="onboard-card"
        role="dialog"
        aria-modal="true"
        aria-label="Welcome"
      >
        <header>
          <p className="eyebrow">CS Mail</p>
          <h2>Welcome to your inbox</h2>
          <p>A fast, focused mail client built for getting through your day.</p>
        </header>
        <ul className="onboard-points">
          {points.map(({ icon: Icon, text }) => (
            <li key={text}>
              <Icon size={16} />
              {text}
            </li>
          ))}
        </ul>
        {step === 'setup' ? (
          <form onSubmit={finishSetup}>
            <label>
              Your name
              <input
                value={name}
                onChange={(event) => setName(event.target.value)}
                placeholder="e.g. Alex Morgan"
                maxLength={80}
                disabled={saving}
              />
            </label>
            {error && <p className="settings-hint">{error}</p>}
            <footer>
              <button className="primary-button" type="submit" disabled={saving}>
                {saving ? 'Setting up…' : 'Enter my inbox'}
              </button>
            </footer>
          </form>
        ) : (
          <footer>
            <button className="primary-button" autoFocus onClick={dismiss}>
              Get started
            </button>
            <button className="secondary-button" onClick={dismiss}>
              Skip for now
            </button>
          </footer>
        )}
      </section>
    </div>
  )
}
