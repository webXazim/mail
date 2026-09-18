import { lazy, Suspense, useEffect, useRef, useState, type CSSProperties } from 'react'
import { Outlet, useLocation, useNavigate } from 'react-router-dom'
import { WifiOff, X } from 'lucide-react'
import { folderFromPath, folderPath, folderSlug } from '../../lib/mail'
import { useMail } from '../../state/mail/MailContext'
import { useFocusTrap } from '../../hooks/useFocusTrap'
import { CommandPalette } from '../CommandPalette'
import { Onboarding } from '../Onboarding'
import { Sidebar } from './Sidebar'
import { Topbar } from './Topbar'

const Composer = lazy(() => import('../Composer').then((module) => ({ default: module.Composer })))

const goShortcuts: Record<string, string> = {
  i: folderSlug.Inbox,
  s: folderSlug.Starred,
  a: folderSlug['All Mail'],
  t: folderSlug.Sent,
  d: folderSlug.Drafts,
  u: folderSlug.Unread,
  z: folderSlug.Snoozed,
  e: folderSlug.Archive,
  p: folderSlug.Spam,
  r: folderSlug.Trash,
}

export function MailLayout() {
  const location = useLocation()
  const navigate = useNavigate()
  const {
    toasts,
    dismissToast,
    unsend,
    undoAction,
    composeOpen,
    composerInitial,
    openCompose,
    closeCompose,
    handleSent,
    undoSecondsLeft,
  } = useMail()
  const [mobile, setMobile] = useState(false)
  const [sidebarWidth, setSidebarWidth] = useState(242)
  const [helpOpen, setHelpOpen] = useState(false)
  const [paletteOpen, setPaletteOpen] = useState(false)
  const [offline, setOffline] = useState(() => !navigator.onLine)
  const helpRef = useRef<HTMLElement>(null)
  const gPendingRef = useRef(false)
  const pathnameRef = useRef(location.pathname)
  useFocusTrap(helpRef, helpOpen)
  useEffect(() => {
    pathnameRef.current = location.pathname
  }, [location.pathname])
  const folder = folderFromPath(location.pathname)
  const threadOpen = Boolean(location.pathname.match(/\/thread\/([^/]+)/)?.[1])
  const adminOpen = location.pathname.endsWith('/admin')

  useEffect(() => {
    const online = () => setOffline(false)
    const offlineEvent = () => setOffline(true)
    window.addEventListener('online', online)
    window.addEventListener('offline', offlineEvent)
    return () => {
      window.removeEventListener('online', online)
      window.removeEventListener('offline', offlineEvent)
    }
  }, [])

  useEffect(() => {
    const handle = (event: globalThis.KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'k') {
        event.preventDefault()
        setPaletteOpen((value) => !value)
        return
      }
      const target = event.target as HTMLElement
      const typing =
        target.tagName === 'INPUT' || target.tagName === 'TEXTAREA' || target.isContentEditable
      if (event.key.toLowerCase() === 'c' && !typing) {
        event.preventDefault()
        openCompose()
      }
      if (event.key === '?') {
        event.preventDefault()
        setHelpOpen((value) => !value)
      }
      if (event.key === 'Escape') {
        setHelpOpen(false)
        setPaletteOpen(false)
        closeCompose()
        setMobile(false)
        if (
          pathnameRef.current.endsWith('/settings') ||
          pathnameRef.current.endsWith('/notifications') ||
          pathnameRef.current.endsWith('/labels') ||
          pathnameRef.current.endsWith('/billing') ||
          pathnameRef.current.endsWith('/admin') ||
          pathnameRef.current.endsWith('/audit-log')
        )
          navigate(`/mail/${folderPath(folder)}`)
      }
      if (!typing) {
        if (event.key.toLowerCase() === 'g' && !gPendingRef.current) {
          gPendingRef.current = true
          event.preventDefault()
          window.setTimeout(() => {
            gPendingRef.current = false
          }, 1500)
          return
        }
        if (gPendingRef.current) {
          gPendingRef.current = false
          const slug = goShortcuts[event.key.toLowerCase()]
          if (slug) {
            event.preventDefault()
            navigate(`/mail/${slug}`)
          }
        }
      }
    }
    window.addEventListener('keydown', handle)
    return () => window.removeEventListener('keydown', handle)
  }, [folder, adminOpen, navigate, openCompose, closeCompose])

  return (
    <div
      className={`app ${threadOpen ? 'app--thread' : ''}`}
      style={{ '--sidebar-width': `${sidebarWidth}px` } as CSSProperties}
    >
      <a className="skip-link" href="#mail-main">
        Skip to content
      </a>
      <Sidebar
        mobile={mobile}
        onCloseMobile={() => setMobile(false)}
        onWidthChange={setSidebarWidth}
        onCompose={openCompose}
      />
      <div
        className={`sidebar-scrim ${mobile ? '' : 'sidebar-scrim--hidden'}`}
        onClick={() => setMobile(false)}
        aria-hidden="true"
      />
      <main id="mail-main" tabIndex={-1} className={`main ${threadOpen ? 'thread-main' : ''}`}>
        {offline && (
          <div className="offline-banner" role="status">
            <WifiOff size={14} />
            You're offline — messages are saved locally and will sync when you reconnect.
          </div>
        )}
        <Topbar
          onOpenMobile={() => setMobile(true)}
          mobileOpen={mobile}
          onHelp={() => setHelpOpen(true)}
        />
        <Outlet key={location.pathname} />
      </main>
      {toasts.length > 0 && (
        <div className="toast-stack" aria-live="polite">
          {toasts.map((toast) => (
            <div className="toast" role="status" key={toast.id}>
              <span>{toast.message}</span>
              {toast.canUndoAction && (
                <button
                  className="toast-action"
                  onClick={() => {
                    undoAction()
                    dismissToast(toast.id)
                  }}
                >
                  Undo
                </button>
              )}
              {toast.canUndoSend && undoSecondsLeft > 0 && (
                <button
                  className="toast-action"
                  onClick={() => {
                    unsend()
                    dismissToast(toast.id)
                  }}
                >
                  Undo · {undoSecondsLeft}s
                </button>
              )}
            </div>
          ))}
        </div>
      )}
      {helpOpen && (
        <div className="help-layer" onClick={() => setHelpOpen(false)}>
          <section
            className="help-panel"
            role="dialog"
            aria-modal="true"
            aria-labelledby="help-title"
            ref={helpRef}
            onClick={(event) => event.stopPropagation()}
          >
            <header>
              <div>
                <p className="eyebrow">Harbor Mail</p>
                <h2 id="help-title">Keyboard shortcuts</h2>
              </div>
              <button
                type="button"
                className="icon-button"
                aria-label="Close shortcuts"
                onClick={() => setHelpOpen(false)}
              >
                <X size={17} />
              </button>
            </header>
            <dl className="help-list">
              <div className="help-group">
                <dt>Compose</dt>
                <dd>
                  <kbd>c</kbd>
                </dd>
              </div>
              <div className="help-group">
                <dt>Search mail</dt>
                <dd>
                  <kbd>/</kbd>
                </dd>
              </div>
              <div className="help-group">
                <dt>Command palette</dt>
                <dd>
                  <kbd>Ctrl</kbd> <em>+</em> <kbd>K</kbd>
                </dd>
              </div>
              <div className="help-group-divider" aria-hidden="true" />
              <div>
                <dt>Next message</dt>
                <dd>
                  <kbd>j</kbd>
                </dd>
              </div>
              <div>
                <dt>Previous message</dt>
                <dd>
                  <kbd>k</kbd>
                </dd>
              </div>
              <div>
                <dt>Open message</dt>
                <dd>
                  <kbd>Enter</kbd>
                </dd>
              </div>
              <div>
                <dt>Select message</dt>
                <dd>
                  <kbd>x</kbd>
                </dd>
              </div>
              <div className="help-group-divider" aria-hidden="true" />
              <div className="help-group">
                <dt>Archive</dt>
                <dd>
                  <kbd>e</kbd>
                </dd>
              </div>
              <div className="help-group">
                <dt>Delete</dt>
                <dd>
                  <kbd>#</kbd>
                </dd>
              </div>
              <div>
                <dt>Star</dt>
                <dd>
                  <kbd>s</kbd>
                </dd>
              </div>
              <div>
                <dt>Undo</dt>
                <dd>
                  <kbd>z</kbd>
                </dd>
              </div>
              <div className="help-group-divider" aria-hidden="true" />
              <div className="help-group">
                <dt>Go straight to a folder</dt>
                <dd>
                  <kbd>g</kbd> then <kbd>i</kbd> <em>/</em> <kbd>s</kbd> <em>/</em> <kbd>a</kbd>{' '}
                  <em>/</em> <kbd>t</kbd> <em>/</em> <kbd>d</kbd> <em>/</em> <kbd>u</kbd> <em>/</em>{' '}
                  <kbd>z</kbd> <em>/</em> <kbd>e</kbd> <em>/</em> <kbd>p</kbd> <em>/</em>{' '}
                  <kbd>r</kbd>
                </dd>
              </div>
              <div className="help-group-divider" aria-hidden="true" />
              <div className="help-group">
                <dt>Reply / Reply all / Forward</dt>
                <dd>
                  <kbd>r</kbd> <em>/</em> <kbd>a</kbd> <em>/</em> <kbd>f</kbd>
                </dd>
              </div>
              <div>
                <dt>Back to folder</dt>
                <dd>
                  <kbd>u</kbd>
                </dd>
              </div>
              <div>
                <dt>Close dialog</dt>
                <dd>
                  <kbd>Esc</kbd>
                </dd>
              </div>
            </dl>
          </section>
        </div>
      )}
      <CommandPalette
        open={paletteOpen}
        onClose={() => setPaletteOpen(false)}
        onNavigate={(path) => navigate(path)}
        onCompose={openCompose}
        onOpenSettings={() => navigate('/mail/settings')}
        onOpenNotifications={() => navigate('/mail/notifications')}
        onOpenContacts={() => navigate('/mail/contacts')}
        onOpenAdmin={() => navigate('/mail/admin')}
        onOpenCalendar={() => navigate('/mail/calendar')}
        onToggleHelp={() => setHelpOpen(true)}
      />
      <Onboarding />
      {composeOpen && (
        <Suspense fallback={null}>
          <Composer
            close={closeCompose}
            onSent={handleSent}
            initialDraft={composerInitial ?? undefined}
          />
        </Suspense>
      )}
    </div>
  )
}
