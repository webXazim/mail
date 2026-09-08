import { Bell, Check, ShieldCheck, X } from 'lucide-react'
import { useRef, useState } from 'react'
import type { ComponentType } from 'react'
import { useFocusTrap } from '../hooks/useFocusTrap'
import { notificationsApi, type NotificationItem } from '../services/notifications'

const icons: Record<NotificationItem['icon'], ComponentType<{ size?: number }>> = { mention: Bell, security: ShieldCheck, scheduled: Check }

export function Notifications({ close }: { close: () => void }) {
  const [items, setItems] = useState<NotificationItem[]>(() => notificationsApi.load())
  const panelRef = useRef<HTMLElement>(null)
  useFocusTrap(panelRef)
  const update = (next: NotificationItem[]) => { setItems(next); notificationsApi.save(next) }
  return (
    <div className="notifications-layer" role="presentation">
      <section ref={panelRef} className="notifications-panel" role="dialog" aria-modal="true" aria-labelledby="notifications-title">
        <header>
          <div><p className="eyebrow">Harbor Mail</p><h2 id="notifications-title">Notifications</h2></div>
          <span>
            <button type="button" className="text-button" onClick={() => update(items.map(item => ({ ...item, unread: false })))}>Mark all read</button>
            <button type="button" className="icon-button" onClick={close} aria-label="Close notifications"><X size={17}/></button>
          </span>
        </header>
        <div className="notification-list">
          {items.length ? items.map(item => {
            const Icon = icons[item.icon]
            return (
              <article className={item.unread ? 'notification notification--unread' : 'notification'} key={item.id}>
                <span className="notification-icon"><Icon size={16}/></span>
                <span><strong>{item.title}</strong><small>{item.detail}</small></span>
                {item.unread && <button type="button" className="notification-unread" aria-label={`Mark ${item.title} as read`} onClick={() => update(items.map(candidate => candidate.id === item.id ? { ...candidate, unread: false } : candidate))}/>}
                <button type="button" className="notification-dismiss" aria-label={`Dismiss ${item.title}`} onClick={() => update(items.filter(candidate => candidate.id !== item.id))}><X size={14}/></button>
              </article>
            )
          }) : (
            <div className="notifications-empty"><Bell size={20} /><strong>You&apos;re all caught up</strong><span>No notifications right now.</span></div>
          )}
        </div>
      </section>
    </div>
  )
}
