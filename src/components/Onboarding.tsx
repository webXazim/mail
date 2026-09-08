import { useEffect, useRef, useState } from 'react'
import { Bell, Inbox, Keyboard, Zap } from 'lucide-react'
import { useFocusTrap } from '../hooks/useFocusTrap'

const onboardedKey = 'harbor-mail:onboarded'

export function Onboarding() {
  const [open, setOpen] = useState(() => localStorage.getItem(onboardedKey) !== '1')
  const cardRef = useRef<HTMLElement>(null)
  useFocusTrap(cardRef)
  const dismiss = () => { localStorage.setItem(onboardedKey, '1'); setOpen(false) }
  useEffect(() => {
    if (!open) return
    const esc = (event: KeyboardEvent) => { if (event.key === 'Escape') dismiss() }
    window.addEventListener('keydown', esc)
    return () => window.removeEventListener('keydown', esc)
  })
  if (!open) return null
  return (
    <div className="onboard-layer">
      <section ref={cardRef} className="onboard-card" role="dialog" aria-modal="true" aria-label="Welcome">
        <header>
          <p className="eyebrow">Harbor Mail</p>
          <h2>Welcome to your inbox</h2>
          <p>A fast, focused mail client built for getting through your day.</p>
        </header>
        <ul className="onboard-points">
          <li><Inbox size={16} />Conversations grouped, priorities surfaced</li>
          <li><Zap size={16} />Send on your schedule — queue, snooze, undo</li>
          <li><Keyboard size={16} />Drive the whole client from the keyboard</li>
          <li><Bell size={16} />Notifications that respect your focus</li>
        </ul>
        <footer>
          <button className="primary-button" autoFocus onClick={dismiss}>Get started</button>
          <button className="secondary-button" onClick={dismiss}>Skip for now</button>
        </footer>
      </section>
    </div>
  )
}