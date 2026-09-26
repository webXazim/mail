import { useEffect, useRef, useState } from 'react'
import { AlertTriangle, X } from 'lucide-react'
import { useFocusTrap } from '../hooks/useFocusTrap'

type Props = {
  open: boolean
  title: string
  description: string
  confirmLabel: string
  onConfirm: () => void | Promise<void>
  onClose: () => void
  busy?: boolean
  danger?: boolean
  verificationText?: string
  details?: string[]
}

export function ConfirmDialog({ open, title, description, confirmLabel, onConfirm, onClose, busy = false, danger = false, verificationText, details = [] }: Props) {
  const dialogRef = useRef<HTMLElement | null>(null)
  const [verification, setVerification] = useState('')
  useFocusTrap(dialogRef, open, verificationText ? '[data-confirm-verification]' : '[data-confirm-primary]')
  useEffect(() => {
    if (!open) return
    const onKeyDown = (event: KeyboardEvent) => { if (event.key === 'Escape' && !busy) onClose() }
    document.addEventListener('keydown', onKeyDown)
    return () => document.removeEventListener('keydown', onKeyDown)
  }, [open, busy, onClose])
  if (!open) return null
  const verified = !verificationText || verification.trim() === verificationText
  return <div className="platform-modal-layer" role="presentation" onMouseDown={(event) => { if (event.target === event.currentTarget && !busy) onClose() }}>
    <section ref={dialogRef} className={`platform-modal${danger ? ' platform-modal--danger' : ''}`} role="dialog" aria-modal="true" aria-labelledby="platform-confirm-title">
      <header className="platform-modal__header"><div className="platform-modal__title">{danger && <AlertTriangle size={18} aria-hidden="true" />}<h2 id="platform-confirm-title">{title}</h2></div><button type="button" className="icon-button" aria-label="Close" onClick={onClose} disabled={busy}><X size={17}/></button></header>
      <div className="platform-modal__body"><p>{description}</p>{details.length > 0 && <ul className="platform-modal__details">{details.map((item) => <li key={item}>{item}</li>)}</ul>}{verificationText && <label className="platform-modal__verification"><span>Type <strong>{verificationText}</strong> to confirm</span><input data-confirm-verification value={verification} onChange={(event) => setVerification(event.target.value)} autoComplete="off" spellCheck={false} disabled={busy}/></label>}</div>
      <footer className="platform-modal__actions"><button type="button" className="secondary-button" onClick={onClose} disabled={busy}>Cancel</button><button type="button" data-confirm-primary className={danger ? 'danger-button' : 'primary-button'} onClick={() => void onConfirm()} disabled={busy || !verified}>{busy ? 'Working…' : confirmLabel}</button></footer>
    </section>
  </div>
}
