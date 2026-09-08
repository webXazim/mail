import { Archive, Star, Trash2, X } from 'lucide-react'
import { memo } from 'react'
import type { Mail } from '../types'

type Props = {
  mail: Mail
  checked?: boolean
  active?: boolean
  onClick: (mail: Mail) => void
  onSelect?: (checked: boolean, id: string) => void
  onQuickArchive?: (mail: Mail) => void
  onQuickStar?: (mail: Mail) => void
  onQuickTrash?: (mail: Mail) => void
  onQuickCancel?: (mail: Mail) => void
}

export const MailRow = memo(function MailRow({ mail, checked = false, active = false, onClick, onSelect, onQuickArchive, onQuickStar, onQuickTrash, onQuickCancel }: Props) {
  return (
    <article className={`mail-row ${mail.unread ? 'mail-row--unread' : ''} ${active ? 'mail-row--active' : ''}`} aria-current={active ? 'page' : undefined}>
      <input type="checkbox" checked={checked} disabled={!onSelect} onChange={event => onSelect?.(event.target.checked, mail.id)} onClick={event => event.stopPropagation()} aria-label={`Select ${mail.subject}`} />
      <span className={`avatar avatar--${mail.color}`}>{mail.initials}</span>
      <span className="mail-row__sender"><strong>{mail.sender}</strong><small>{mail.email}</small></span>
      <span className="mail-row__content"><strong>{mail.subject}</strong><small>{mail.preview}</small></span>
      <span className="mail-row__meta">
        <span className="mail-row__quicks" onClick={event => event.stopPropagation()}>
          {onQuickCancel && <button type="button" aria-label={`Cancel ${mail.subject}`} onClick={() => onQuickCancel(mail)}><X size={15} /></button>}
          {onQuickArchive && <button type="button" aria-label={`Archive ${mail.subject}`} onClick={() => onQuickArchive(mail)}><Archive size={15} /></button>}
          {onQuickStar && <button type="button" aria-label={`${mail.starred ? 'Unstar' : 'Star'} ${mail.subject}`} onClick={() => onQuickStar(mail)}><Star size={15} fill={mail.starred ? 'currentColor' : 'none'} /></button>}
          {onQuickTrash && <button type="button" aria-label={`Move ${mail.subject} to trash`} onClick={() => onQuickTrash(mail)}><Trash2 size={15} /></button>}
        </span>
        <time>{mail.time}</time>
        {active && <span className="badge badge--open">Open</span>}
        {mail.label && <span className="badge">{mail.label}</span>}
      </span>
      <button type="button" className="mail-row__hit" aria-label={`Open ${mail.subject}`} onClick={() => onClick(mail)} />
    </article>
  )
})