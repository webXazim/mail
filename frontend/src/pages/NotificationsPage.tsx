import { useCallback, useEffect, useState, type ComponentType } from 'react'
import { useNavigate } from 'react-router-dom'
import {
  ArrowLeft,
  Bell,
  Check,
  CreditCard,
  LifeBuoy,
  Mail as MailIcon,
  ShieldCheck,
  UserRound,
  X,
} from 'lucide-react'
import { notificationsApi, type NotificationItem } from '../services/notifications'

const icons: Record<NotificationItem['icon'], ComponentType<{ size?: number }>> = {
  security: ShieldCheck,
  scheduled: Check,
  mail: MailIcon,
  billing: CreditCard,
  support: LifeBuoy,
  account: UserRound,
}

const timeFmt = (iso: string) =>
  new Date(iso).toLocaleString([], {
    month: 'short',
    day: 'numeric',
    hour: 'numeric',
    minute: '2-digit',
  })

export function NotificationsPage() {
  const navigate = useNavigate()
  const [items, setItems] = useState<NotificationItem[]>(() => notificationsApi.load())
  const [unreadCount, setUnreadCount] = useState(() => items.filter((item) => item.unread).length)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState('')
  const [hasMore, setHasMore] = useState(false)
  const [nextBefore, setNextBefore] = useState<string | null>(null)

  const load = useCallback(async (append = false) => {
    try {
      setError('')
      const page = await notificationsApi.list(append ? nextBefore : null)
      setItems((current) => (append ? [...current, ...page.items] : page.items))
      setUnreadCount(page.unread)
      setHasMore(page.hasMore)
      setNextBefore(page.nextBefore)
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : 'Unable to load notifications')
    } finally {
      setLoading(false)
    }
  }, [nextBefore])

  useEffect(() => {
    void load(false)
  }, []) // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    const refresh = (event: Event) => {
      const detail = (event as CustomEvent<{ payload?: { resource?: string } }>).detail
      if (detail?.payload?.resource === 'notifications') void load(false)
    }
    window.addEventListener('cs-mail-resource-changed', refresh)
    return () => window.removeEventListener('cs-mail-resource-changed', refresh)
  }, [load])

  const markAll = async () => {
    try {
      await notificationsApi.markAllRead()
      setItems((current) => current.map((item) => ({ ...item, unread: false })))
      setUnreadCount(0)
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : 'Unable to update notifications')
    }
  }

  const markRead = async (id: string) => {
    try {
      await notificationsApi.markRead(id)
      setItems((current) =>
        current.map((item) => (item.id === id ? { ...item, unread: false } : item)),
      )
      setUnreadCount((current) => Math.max(0, current - 1))
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : 'Unable to update notification')
    }
  }

  const dismiss = async (id: string) => {
    const target = items.find((item) => item.id === id)
    try {
      await notificationsApi.dismiss(id)
      setItems((current) => current.filter((item) => item.id !== id))
      if (target?.unread) setUnreadCount((current) => Math.max(0, current - 1))
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : 'Unable to dismiss notification')
    }
  }

  return (
    <div className="settings-page" role="region" aria-label="Notifications">
      <header className="calendar-head">
        <div>
          <p className="eyebrow">CS Mail</p>
          <h1>Notifications</h1>
        </div>
        <div className="calendar-head__actions">
          <button
            type="button"
            className="secondary-button"
            onClick={() => void markAll()}
            disabled={unreadCount === 0}
          >
            <Check size={14} />
            Mark all read
          </button>
          <button type="button" className="secondary-button" onClick={() => navigate('/mail/inbox')}>
            <ArrowLeft size={14} />
            Back to inbox
          </button>
        </div>
      </header>

      {error && <p className="form-error" role="alert">{error}</p>}

      <div className="settings-section">
        {loading && items.length === 0 ? (
          <div className="list-state"><div className="loading-spinner" /><span>Loading notifications…</span></div>
        ) : items.length ? (
          <div className="notification-list">
            {items.map((item) => {
              const Icon = icons[item.icon]
              return (
                <article
                  className={item.unread ? 'notification notification--unread' : 'notification'}
                  key={item.id}
                >
                  <span className="notification-icon"><Icon size={16} /></span>
                  <button
                    type="button"
                    className="notification-copy"
                    onClick={() => item.actionUrl && navigate(item.actionUrl)}
                  >
                    <strong>{item.title}</strong>
                    <small>{item.detail}</small>
                    <small>{timeFmt(item.createdAt)}</small>
                  </button>
                  {item.unread && (
                    <button
                      type="button"
                      className="notification-unread"
                      aria-label={`Mark ${item.title} as read`}
                      onClick={() => void markRead(item.id)}
                    />
                  )}
                  <button
                    type="button"
                    className="notification-dismiss"
                    aria-label={`Dismiss ${item.title}`}
                    onClick={() => void dismiss(item.id)}
                  >
                    <X size={14} />
                  </button>
                </article>
              )
            })}
            {hasMore && (
              <button type="button" className="secondary-button" onClick={() => void load(true)}>
                Load older
              </button>
            )}
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
        <button type="button" className="primary-button" onClick={() => navigate('/mail/inbox')}>Done</button>
      </footer>
    </div>
  )
}
