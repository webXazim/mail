import { useState, type ComponentType } from 'react'
import { useNavigate } from 'react-router-dom'
import { ArrowLeft, Bell, Check, Mail as MailIcon, ShieldCheck, X } from 'lucide-react'
import { notificationsApi, type NotificationItem } from '../services/notifications'

const icons: Record<NotificationItem['icon'], ComponentType<{ size?: number }>> = {
  mention: Bell,
  security: ShieldCheck,
  scheduled: Check,
  mail: MailIcon,
}

export function NotificationsPage() {
  const navigate = useNavigate()
  const [items, setItems] = useState<NotificationItem[]>(() => notificationsApi.load())
  const update = (next: NotificationItem[]) => {
    setItems(next)
    notificationsApi.save(next)
  }
  const unreadCount = items.filter((item) => item.unread).length

  return (
    <div className="settings-page" role="region" aria-label="Notifications">
      <header className="calendar-head">
        <div>
          <p className="eyebrow">Harbor Mail</p>
          <h1>Notifications</h1>
        </div>
        <div className="calendar-head__actions">
          <button
            type="button"
            className="secondary-button"
            onClick={() => update(items.map((item) => ({ ...item, unread: false })))}
            disabled={unreadCount === 0}
          >
            <Check size={14} />
            Mark all read
          </button>
          <button
            type="button"
            className="secondary-button"
            onClick={() => navigate('/mail/inbox')}
          >
            <ArrowLeft size={14} />
            Back to inbox
          </button>
        </div>
      </header>

      <div className="settings-section">
        {items.length ? (
          <div className="notification-list">
            {items.map((item) => {
              const Icon = icons[item.icon]
              return (
                <article
                  className={item.unread ? 'notification notification--unread' : 'notification'}
                  key={item.id}
                >
                  <span className="notification-icon">
                    <Icon size={16} />
                  </span>
                  <span>
                    <strong>{item.title}</strong>
                    <small>{item.detail}</small>
                  </span>
                  {item.unread && (
                    <button
                      type="button"
                      className="notification-unread"
                      aria-label={`Mark ${item.title} as read`}
                      onClick={() =>
                        update(
                          items.map((candidate) =>
                            candidate.id === item.id ? { ...candidate, unread: false } : candidate,
                          ),
                        )
                      }
                    />
                  )}
                  <button
                    type="button"
                    className="notification-dismiss"
                    aria-label={`Dismiss ${item.title}`}
                    onClick={() => update(items.filter((candidate) => candidate.id !== item.id))}
                  >
                    <X size={14} />
                  </button>
                </article>
              )
            })}
          </div>
        ) : (
          <div className="notifications-empty">
            <Bell size={20} />
            <strong>You&apos;re all caught up</strong>
            <span>No notifications right now.</span>
          </div>
        )}
      </div>

      <footer>
        <button type="button" className="primary-button" onClick={() => navigate('/mail/inbox')}>
          Done
        </button>
      </footer>
    </div>
  )
}
