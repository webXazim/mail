import { useEffect, useMemo, useState } from 'react'
import {
  Check,
  Building2,
  CalendarDays,
  ChevronDown,
  CircleHelp,
  Bell,
  CreditCard,
  Folder,
  Layers,
  LayoutTemplate,
  LogOut,
  Server,
  Settings2,
  SquarePen,
  UserRound,
  X,
} from 'lucide-react'
import { useLocation, useNavigate, useSearchParams } from 'react-router-dom'
import {
  folderFromPath,
  folderSlug,
  folders,
  getFolderCounts,
  navGroupTitles,
} from '../../lib/mail'
import { useMail } from '../../state/mail/MailContext'
import { draftsApi } from '../../services/drafts'
import { foldersApi } from '../../services/folders'
import { labelsApi } from '../../services/labels'
import { settingsApi } from '../../services/settings'
import { authApi } from '../../services/auth'
import { currentVirtualCounts, isRemoteMail } from '../../services/remote-mail'
import { displayNameOf, profileApi, useProfile, useRole } from '../../services/profile'
import { primaryAccountId, unifiedViewId } from '../../services/accounts'
import { ManageFolders } from '../ManageFolders'
import { BrandIdentity } from '../BrandIdentity'
import { isLocalAdminOrigin } from '../../lib/admin-origin'
import type { Mailbox } from '../../types'

type SidebarProps = {
  mobile: boolean
  onCloseMobile: () => void
  onWidthChange: (width: number) => void
  onCompose: () => void
}

const clampWidth = (value: number) => Math.max(210, Math.min(360, value))
const isRemoteMode = (mailboxes: { id: string }[]) => mailboxes.length > 0

export function Sidebar({ mobile, onCloseMobile, onWidthChange, onCompose }: SidebarProps) {
  const { pathname } = useLocation()
  const [searchParams] = useSearchParams()
  const navigate = useNavigate()
  const { mailbox, scheduledCount, accounts, activeAccount, setActiveAccount, remoteMailboxes } = useMail()
  const role = useRole()
  const signedInProfile = useProfile()
  const platformAdmin = role === 'admin' && isLocalAdminOrigin()
  const platformOnly = isRemoteMail() && signedInProfile?.has_mailbox !== true
  const planActive = signedInProfile?.subscription_status === 'active' || signedInProfile?.subscription_status === 'trial'
  const folder = folderFromPath(pathname)
  const query = searchParams.get('q') || ''
  const counts = useMemo(() => {
    if (!isRemoteMode(remoteMailboxes)) return getFolderCounts(mailbox)
    const byRole = new Map(remoteMailboxes.filter((item) => item.role).map((item) => [item.role, item]))
    const trash = byRole.get('trash')
    const virtual = currentVirtualCounts()
    return {
      Inbox: byRole.get('inbox')?.total ?? 0,
      Unread: virtual.unread,
      Starred: virtual.starred,
      Snoozed: mailbox.filter((mail) => mail.folder === 'Snoozed').length,
      Sent: byRole.get('sent')?.total ?? 0,
      Drafts: byRole.get('drafts')?.total ?? 0,
      'All Mail': virtual.all,
      Archive: byRole.get('archive')?.total ?? 0,
      Spam: byRole.get('junk')?.total ?? 0,
      Trash: trash?.total ?? 0,
    }
  }, [mailbox, remoteMailboxes])
  const profile = useMemo(() => {
    const active = accounts.find((account) => account.id === activeAccount) ?? accounts[0]
    const displayName = signedInProfile && active.id === primaryAccountId
      ? displayNameOf(signedInProfile)
      : active.id === primaryAccountId && authApi.isDemo()
        ? settingsApi.load().displayName || active.name
        : active.name
    const clear = displayName.trim().split(/\s+/)
    const initials =
      clear
        .map((part) => part[0]?.toUpperCase() ?? '')
        .slice(0, 2)
        .join('') || '?'
    return { displayName, initials, email: active.email, color: active.color }
  }, [accounts, activeAccount, signedInProfile])
  const storage = useMemo(() => {
    if (signedInProfile?.storage) {
      const used = signedInProfile.storage.used_bytes / 1024 ** 3
      const total = signedInProfile.storage.total_bytes / 1024 ** 3
      return {
        used: used >= 10 ? used.toFixed(0) : used.toFixed(1),
        total: total >= 10 ? total.toFixed(0) : total.toFixed(1),
        percent: Math.min(100, Math.max(0, Math.round(signedInProfile.storage.pct))),
      }
    }
    if (!authApi.isDemo()) return { used: '—', total: '—', percent: 0 }
    const attachments = mailbox.filter((mail) => mail.attachment).length
    const used = 0.4 + mailbox.filter((mail) => mail.folder !== 'Trash').length * 0.002 + attachments * 0.012
    return { used: used.toFixed(1), total: '15', percent: Math.min(100, Math.round((used / 15) * 100)) }
  }, [mailbox, signedInProfile])
  const [resizing, setResizing] = useState(false)
  const [profileOpen, setProfileOpen] = useState(false)
  const [moreFoldersOpen, setMoreFoldersOpen] = useState(false)
  const [labelsOpen, setLabelsOpen] = useState(false)
  const [manageFoldersOpen, setManageFoldersOpen] = useState(false)
  const labels = labelsApi.list()
  const customFolders = foldersApi.list()
  useEffect(() => {
    const refreshStorage = (incoming: Event) => {
      const detail = (incoming as CustomEvent<{ kind?: string }>).detail
      if (detail?.kind === 'quota') void profileApi.refresh()
    }
    window.addEventListener('cs-mail-realtime', refreshStorage)
    return () => window.removeEventListener('cs-mail-realtime', refreshStorage)
  }, [])
  useEffect(() => {
    if (!profileOpen) return
    const close = () => setProfileOpen(false)
    const esc = (event: KeyboardEvent) => {
      if (event.key === 'Escape') setProfileOpen(false)
    }
    window.addEventListener('click', close)
    window.addEventListener('keydown', esc)
    return () => {
      window.removeEventListener('click', close)
      window.removeEventListener('keydown', esc)
    }
  }, [profileOpen])
  useEffect(() => {
    if (!resizing) return
    const move = (event: MouseEvent) => onWidthChange(clampWidth(event.clientX))
    const stop = () => setResizing(false)
    window.addEventListener('mousemove', move)
    window.addEventListener('mouseup', stop)
    return () => {
      window.removeEventListener('mousemove', move)
      window.removeEventListener('mouseup', stop)
    }
  }, [resizing, onWidthChange])
  const inMoreDrawer = (label: Mailbox) =>
    folders.some(
      (item) => item.label === label && (item.group === 'more' || item.label === 'Scheduled'),
    )
  const goFolder = (label: Mailbox) => {
    navigate(`/mail/${folderSlug[label]}`)
    setMoreFoldersOpen(inMoreDrawer(label))
    setLabelsOpen(false)
    onCloseMobile()
  }
  const goLabel = (label: string) => {
    const target = folder in folderSlug ? folderSlug[folder as Mailbox] : 'inbox'
    navigate(`/mail/${target}?q=${encodeURIComponent(`label:${label}`)}`)
    setLabelsOpen(true)
    onCloseMobile()
  }
  const goCustomFolder = (id: string) => {
    navigate(`/mail/folders/${encodeURIComponent(id)}`)
    onCloseMobile()
  }
  const closeMenu = () => setProfileOpen(false)
  const goAccount = (path: string) => {
    navigate(path)
    closeMenu()
    onCloseMobile()
  }
  const switchAccount = (accountId: string) => {
    setActiveAccount(accountId)
    closeMenu()
    onCloseMobile()
  }
  const signOut = () => {
    closeMenu()
    void authApi.logout().finally(() => navigate('/login'))
  }
  const secondaryFolderActive =
    pathname.startsWith('/mail/all') ||
    pathname.startsWith('/mail/archive') ||
    pathname.startsWith('/mail/spam') ||
    pathname.startsWith('/mail/trash') ||
    pathname.startsWith('/mail/scheduled')
  const labelsActive = query.toLowerCase().startsWith('label:')
  const renderFolder = ({ label, icon: Icon }: (typeof folders)[number]) => {
    const count =
      label === 'Scheduled'
        ? scheduledCount
        : label === 'Drafts'
          ? draftsApi.load()
            ? 1
            : 0
          : (counts[label] ?? 0)
    const active = folder === label
    return (
      <button
        key={label}
        className={`folder-link ${active ? 'folder-link--active' : ''}`}
        aria-current={active ? 'page' : undefined}
        onClick={() => goFolder(label)}
      >
        <Icon size={17} />
        <span>{label}</span>
        {count > 0 && <b>{count}</b>}
      </button>
    )
  }
  if (platformOnly) {
    return (
      <aside id="sidebar" className={`sidebar ${mobile ? 'sidebar--open' : ''}`} aria-label="Business navigation">
        <div className="brand-row">
          <a className="brand" href="/" aria-label="CS Mail home"><BrandIdentity /></a>
          <button className="icon-button sidebar-close" onClick={onCloseMobile} aria-label="Close navigation"><X size={17} /></button>
        </div>
        <section className="sidebar-workspace">
          {platformAdmin && <>
            <p className="nav-heading">Platform</p>
            <button className={`folder-link ${pathname === '/mail/admin/control-plane' || pathname === '/mail/admin' ? 'folder-link--active' : ''}`} onClick={() => goAccount('/mail/admin/control-plane')}>
              <Server size={17} /><span>Control plane</span>
            </button>
            <button className={`folder-link ${pathname === '/mail/admin/operations' ? 'folder-link--active' : ''}`} onClick={() => goAccount('/mail/admin/operations')}>
              <Settings2 size={17} /><span>Operations</span>
            </button>
            <button className={`folder-link ${pathname.startsWith('/mail/admin/billing') ? 'folder-link--active' : ''}`} onClick={() => goAccount('/mail/admin/billing')}>
              <CreditCard size={17} /><span>Payments &amp; plans</span>
            </button>
          </>}
          <p className="nav-heading">Your business</p>
          <button className={`folder-link ${pathname.startsWith('/mail/billing') || pathname.startsWith('/mail/pricing') ? 'folder-link--active' : ''}`} onClick={() => goAccount('/mail/billing')}>
            <CreditCard size={17} /><span>Plans</span>
          </button>
          {planActive && <>
            <button className={`folder-link ${pathname.startsWith('/mail/business') ? 'folder-link--active' : ''}`} onClick={() => goAccount('/mail/business')}>
              <Building2 size={17} /><span>Business admin · domains</span>
            </button>
            <button className={`folder-link ${pathname.startsWith('/mail/notifications') ? 'folder-link--active' : ''}`} onClick={() => goAccount('/mail/notifications')}>
              <Bell size={17} /><span>Notifications</span>
            </button>
            <button className={`folder-link ${pathname.startsWith('/mail/settings') ? 'folder-link--active' : ''}`} onClick={() => goAccount('/mail/settings')}>
              <Settings2 size={17} /><span>Account settings</span>
            </button>
          </>}
        </section>
        <div className="sidebar-footer">
          <div className="business-placeholder">
            <strong>{platformAdmin ? 'Platform access' : planActive ? 'No hosted mailbox yet' : 'Plan setup'}</strong>
            <p>{platformAdmin ? 'Customer domains and mailboxes are managed inside each business workspace.' : planActive ? 'Verify a business domain before mail features are enabled.' : 'Choose a plan and complete activation to start domain setup.'}</p>
          </div>
          <div className="profile-wrap">
            <div className="profile">
              <span className={`avatar avatar--${profile.color}`}>{profile.initials}</span>
              <span><strong>{profile.displayName}</strong><small>{signedInProfile?.login_email || profile.email}</small></span>
            </div>
            <button type="button" className="folder-link" onClick={signOut}><LogOut size={16} /><span>Sign out</span></button>
          </div>
        </div>
      </aside>
    )
  }

  return (
    <aside
      id="sidebar"
      className={`sidebar ${mobile ? 'sidebar--open' : ''}`}
      aria-label="Mailbox navigation"
    >
      <button
        className="sidebar-resizer"
        aria-label="Resize navigation"
        onMouseDown={() => setResizing(true)}
      />
      <div className="brand-row">
        <a className="brand" href="/" aria-label="CS Mail home">
          <BrandIdentity />
        </a>
        <button
          className="icon-button sidebar-close"
          onClick={onCloseMobile}
          aria-label="Close navigation"
        >
          <X size={17} />
        </button>
      </div>
      <button className="compose-button" onClick={onCompose}>
        <SquarePen size={17} />
        Compose
      </button>
      {(['mail', 'compose'] as const).map((group) => (
        <nav key={group} className="folder-nav" aria-label={navGroupTitles[group]}>
          <p className="nav-heading">{navGroupTitles[group]}</p>
          {folders
            .filter((item) => item.group === group && item.label !== 'Scheduled')
            .map(renderFolder)}
        </nav>
      ))}
      <div
        className={`sidebar-drawer-wrap ${moreFoldersOpen ? 'sidebar-drawer-wrap--expanded' : ''}`}
      >
        <button
          type="button"
          className={`sidebar-drawer-toggle ${moreFoldersOpen || secondaryFolderActive ? 'sidebar-drawer-toggle--active' : ''}`}
          aria-expanded={moreFoldersOpen}
          aria-controls="sidebar-more-folders"
          onClick={() => setMoreFoldersOpen((value) => !value)}
        >
          <span>More</span>
          <ChevronDown size={14} className="sidebar-drawer-toggle__chevron" />
        </button>
        <div className="sidebar-drawer-collapse">
          <div id="sidebar-more-folders" className="sidebar-drawer">
            <nav className="folder-nav" aria-label="More">
              {folders
                .filter((item) => item.group === 'more' || item.label === 'Scheduled')
                .map(renderFolder)}
            </nav>
          </div>
        </div>
      </div>
      <div className={`sidebar-drawer-wrap ${labelsOpen ? 'sidebar-drawer-wrap--expanded' : ''}`}>
        <div
          className={`sidebar-drawer-row ${labelsOpen || labelsActive ? 'sidebar-drawer-row--active' : ''}`}
        >
          <button
            type="button"
            className="sidebar-drawer-toggle"
            aria-expanded={labelsOpen}
            aria-controls="sidebar-labels"
            onClick={() => setLabelsOpen((value) => !value)}
          >
            <span>Labels</span>
            <ChevronDown size={14} className="sidebar-drawer-toggle__chevron" />
          </button>
          {labelsOpen && (
            <button
              type="button"
              className="sidebar-drawer-toggle__settings"
              aria-label="Manage labels"
              onClick={() => {
                setLabelsOpen(false)
                navigate('/mail/labels')
              }}
            >
              <Settings2 size={14} />
            </button>
          )}
        </div>
        <div className="sidebar-drawer-collapse">
          <div id="sidebar-labels" className="sidebar-drawer">
            <nav className="folder-nav" aria-label="Labels">
              {labels.map(({ name, color }) => (
                <button
                  className={`folder-link ${query.toLowerCase() === `label:${name.toLowerCase()}` ? 'folder-link--active' : ''}`}
                  key={name}
                  onClick={() => goLabel(name)}
                >
                  <i className={`label-dot label-dot--${color}`} />
                  {name}
                </button>
              ))}
            </nav>
          </div>
        </div>
      </div>
      <nav className="folder-nav sidebar-custom-folders" aria-label="Folders">
        <div className="nav-heading-row">
          <p className="nav-heading">Folders</p>
          <button
            type="button"
            aria-label="Manage folders"
            onClick={() => setManageFoldersOpen(true)}
          >
            <Settings2 size={13} />
          </button>
        </div>
        {customFolders.map((customFolder) => {
          const count = customFolder.total ?? mailbox.filter((mail) => mail.folder === customFolder.name).length
          const active = pathname.startsWith(`/mail/folders/${encodeURIComponent(customFolder.id)}`)
          return (
            <button
              key={customFolder.id}
              className={`folder-link ${active ? 'folder-link--active' : ''}`}
              aria-current={active ? 'page' : undefined}
              onClick={() => goCustomFolder(customFolder.id)}
            >
              <Folder size={17} />
              <span>{customFolder.name}</span>
              {count > 0 && <b>{count}</b>}
            </button>
          )
        })}
      </nav>
      <section className="sidebar-workspace">
        <p className="nav-heading">Workspace</p>
        <button
          className={`folder-link ${pathname.startsWith('/mail/calendar') ? 'folder-link--active' : ''}`}
          aria-current={pathname.startsWith('/mail/calendar') ? 'page' : undefined}
          onClick={() => {
            navigate('/mail/calendar')
            setMoreFoldersOpen(false)
            setLabelsOpen(false)
            onCloseMobile()
          }}
        >
          <CalendarDays size={17} />
          <span>Calendar</span>
        </button>
        <button
          className={`folder-link ${pathname.startsWith('/mail/contacts') ? 'folder-link--active' : ''}`}
          aria-current={pathname.startsWith('/mail/contacts') ? 'page' : undefined}
          onClick={() => {
            navigate('/mail/contacts')
            setMoreFoldersOpen(false)
            setLabelsOpen(false)
            onCloseMobile()
          }}
        >
          <UserRound size={17} />
          <span>Contacts</span>
        </button>
        <button
          className={`folder-link ${pathname.startsWith('/mail/templates') ? 'folder-link--active' : ''}`}
          aria-current={pathname.startsWith('/mail/templates') ? 'page' : undefined}
          onClick={() => {
            navigate('/mail/templates')
            setMoreFoldersOpen(false)
            setLabelsOpen(false)
            onCloseMobile()
          }}
        >
          <LayoutTemplate size={17} />
          <span>Templates</span>
        </button>
      </section>
      <div className="sidebar-footer">
        <div className="storage">
          <span>Storage</span>
          <strong>
            {storage.used} GB <small>/ {storage.total} GB</small>
          </strong>
          <div className="storage-bar">
            <span style={{ width: `${storage.percent}%` }} />
          </div>
        </div>
        <div className="profile-wrap">
          <button
            className="profile"
            onClick={(event) => {
              event.stopPropagation()
              setProfileOpen((value) => !value)
            }}
            aria-label="Open account menu"
            aria-haspopup="menu"
            aria-expanded={profileOpen}
          >
            <span className={`avatar avatar--${profile.color}`}>{profile.initials}</span>
            <span>
              <strong>{profile.displayName}</strong>
              <small>{profile.email}</small>
            </span>
            <ChevronDown
              size={15}
              className={`profile-chevron ${profileOpen ? 'profile-chevron--open' : ''}`}
            />
          </button>
          {profileOpen && (
            <div className="profile-menu" role="menu">
              <div className="profile-menu__accounts" aria-label="Switch account">
                <button
                  type="button"
                  className={`profile-menu__account-button ${activeAccount === unifiedViewId ? 'profile-menu__account-button--active' : ''}`}
                  aria-label="Switch to unified inbox"
                  onClick={() => switchAccount(unifiedViewId)}
                >
                  <Layers size={14} />
                  <span>Unified inbox</span>
                  {activeAccount === unifiedViewId && <Check size={13} />}
                </button>
                {accounts
                  .filter(
                    (account) =>
                      account.id !== activeAccount &&
                      !(activeAccount === unifiedViewId && account.id === primaryAccountId),
                  )
                  .map((account) => (
                    <button
                      type="button"
                      className={`profile-menu__account-button ${activeAccount === account.id ? 'profile-menu__account-button--active' : ''}`}
                      aria-label={`Switch to ${account.email}`}
                      onClick={() => switchAccount(account.id)}
                      key={account.id}
                    >
                      <span className={`avatar avatar--${account.color}`}>{account.initials}</span>
                      <span>
                        {account.name}
                        <small>{account.email}</small>
                      </span>
                      {activeAccount === account.id && <Check size={13} />}
                    </button>
                  ))}
                <button
                  type="button"
                  className="profile-menu__manage"
                  onClick={() => goAccount('/mail/settings?tab=accounts')}
                >
                  <Settings2 size={13} />
                  Manage accounts
                </button>
              </div>
              <div className="profile-menu__divider" />
              <button
                type="button"
                role="menuitem"
                className="profile-menu__item"
                onClick={() => goAccount('/mail/business')}
              >
                <Building2 size={14} />
                Business
              </button>
              <button
                type="button"
                role="menuitem"
                className="profile-menu__item"
                onClick={() => goAccount('/mail/settings')}
              >
                <Settings2 size={14} />
                Settings
              </button>
              <button
                type="button"
                role="menuitem"
                className="profile-menu__item"
                onClick={() => goAccount('/mail/billing')}
              >
                <CreditCard size={14} />
                Billing
              </button>
              {role === 'admin' && isLocalAdminOrigin() && (
                <button
                  type="button"
                  role="menuitem"
                  className="profile-menu__item"
                  onClick={() => goAccount('/mail/admin/control-plane')}
                >
                  <Server size={14} />
                  Platform control plane
                </button>
              )}
              <button
                type="button"
                role="menuitem"
                className="profile-menu__item"
                onClick={() => {
                  setProfileOpen(false)
                  navigate('/help')
                }}
              >
                <CircleHelp size={14} />
                Help center
              </button>
              <div className="profile-menu__divider" />
              <button
                type="button"
                role="menuitem"
                className="profile-menu__item"
                onClick={signOut}
              >
                <LogOut size={14} />
                Sign out
              </button>
            </div>
          )}
        </div>
      </div>
      {manageFoldersOpen && <ManageFolders close={() => setManageFoldersOpen(false)} />}
    </aside>
  )
}
