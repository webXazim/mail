import { useEffect, useMemo, useState } from 'react'
import { Check, CalendarDays, ChevronDown, CreditCard, Folder, Layers, LogOut, Server, Settings2, SquarePen, UserRound, X } from 'lucide-react'
import { useLocation, useNavigate, useSearchParams } from 'react-router-dom'
import { folderFromPath, folderSlug, folders, getFolderCounts, navGroupTitles } from '../../lib/mail'
import { useMail } from '../../state/mail/MailContext'
import { draftsApi } from '../../services/drafts'
import { foldersApi } from '../../services/folders'
import { labelsApi } from '../../services/labels'
import { settingsApi } from '../../services/settings'
import { authApi } from '../../services/auth'
import { primaryAccountId, unifiedViewId } from '../../services/accounts'
import { ManageFolders } from '../ManageFolders'
import { ManageLabels } from '../ManageLabels'
import type { Mailbox } from '../../types'

type SidebarProps = {
  mobile: boolean
  onCloseMobile: () => void
  onWidthChange: (width: number) => void
  onCompose: () => void
}

const clampWidth = (value: number) => Math.max(210, Math.min(360, value))

export function Sidebar({ mobile, onCloseMobile, onWidthChange, onCompose }: SidebarProps) {
  const { pathname } = useLocation()
  const [searchParams] = useSearchParams()
  const navigate = useNavigate()
  const { mailbox, scheduledCount, accounts, activeAccount, setActiveAccount } = useMail()
  const folder = folderFromPath(pathname)
  const query = searchParams.get('q') || ''
  const counts = useMemo(() => getFolderCounts(mailbox), [mailbox])
  const profile = useMemo(() => {
    const active = accounts.find(account => account.id === activeAccount) ?? accounts[0]
    const displayName = active.id === primaryAccountId ? (settingsApi.load().displayName || active.name) : active.name
    const clear = displayName.trim().split(/\s+/)
    const initials = clear.map(part => part[0]?.toUpperCase() ?? '').slice(0, 2).join('') || 'AM'
    return { displayName, initials, email: active.email, color: active.color }
  }, [accounts, activeAccount])
  const storage = useMemo(() => {
    const attachments = mailbox.filter(mail => mail.attachment).length
    const used = 0.4 + mailbox.filter(mail => mail.folder !== 'Trash').length * 0.002 + attachments * 0.012
    return { used: used.toFixed(1), percent: Math.min(100, Math.round((used / 15) * 100)) }
  }, [mailbox])
  const [resizing, setResizing] = useState(false)
  const [profileOpen, setProfileOpen] = useState(false)
  const [moreFoldersOpen, setMoreFoldersOpen] = useState(false)
  const [labelsOpen, setLabelsOpen] = useState(false)
  const [labels, setLabels] = useState(() => labelsApi.list())
  const [customFolders, setCustomFolders] = useState(() => foldersApi.list())
  const [manageLabelsOpen, setManageLabelsOpen] = useState(false)
  const [manageFoldersOpen, setManageFoldersOpen] = useState(false)
  useEffect(() => {
    if (!profileOpen) return
    const close = () => setProfileOpen(false)
    const esc = (event: KeyboardEvent) => { if (event.key === 'Escape') setProfileOpen(false) }
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
  const inMoreDrawer = (label: Mailbox) => folders.some(item => item.label === label && (item.group === 'more' || item.label === 'Scheduled'))
  const goFolder = (label: Mailbox) => { navigate(`/mail/${folderSlug[label]}`); setMoreFoldersOpen(inMoreDrawer(label)); setLabelsOpen(false); onCloseMobile() }
  const goLabel = (label: string) => {
    const target = folder in folderSlug ? folderSlug[folder as Mailbox] : 'inbox'
    navigate(`/mail/${target}?q=${encodeURIComponent(`label:${label}`)}`)
    setLabelsOpen(true)
    onCloseMobile()
  }
  const goCustomFolder = (id: string) => { navigate(`/mail/folders/${id}`); onCloseMobile() }
  const closeMenu = () => setProfileOpen(false)
  const goAccount = (path: string) => { navigate(path); closeMenu(); onCloseMobile() }
  const switchAccount = (accountId: string) => { setActiveAccount(accountId); closeMenu(); onCloseMobile() }
  const signOut = () => { closeMenu(); void authApi.logout().finally(() => navigate('/login')) }
  const secondaryFolderActive = pathname.startsWith('/mail/all') || pathname.startsWith('/mail/archive') || pathname.startsWith('/mail/spam') || pathname.startsWith('/mail/trash') || pathname.startsWith('/mail/scheduled')
  const labelsActive = query.toLowerCase().startsWith('label:')
  const renderFolder = ({ label, icon: Icon }: typeof folders[number]) => {
    const count = label === 'Scheduled' ? scheduledCount : label === 'Drafts' ? (draftsApi.load() ? 1 : 0) : (counts[label] ?? 0)
    const active = folder === label
    return (
      <button key={label} className={`folder-link ${active ? 'folder-link--active' : ''}`} aria-current={active ? 'page' : undefined} onClick={() => goFolder(label)}>
        <Icon size={17} /><span>{label}</span>{count > 0 && <b>{count}</b>}
      </button>
    )
  }
  return (
    <aside id="sidebar" className={`sidebar ${mobile ? 'sidebar--open' : ''}`} aria-label="Mailbox navigation">
      <button className="sidebar-resizer" aria-label="Resize navigation" onMouseDown={() => setResizing(true)} />
      <div className="brand-row">
        <a className="brand" href="/"><span className="brand-mark">H</span><span>harbor<span>mail</span></span></a>
        <button className="icon-button sidebar-close" onClick={onCloseMobile} aria-label="Close navigation"><X size={17} /></button>
      </div>
      <button className="compose-button" onClick={onCompose}><SquarePen size={17} />Compose</button>
      {(['mail', 'compose'] as const).map(group => (
        <nav key={group} className="folder-nav" aria-label={navGroupTitles[group]}>
          <p className="nav-heading">{navGroupTitles[group]}</p>
          {folders.filter(item => item.group === group && item.label !== 'Scheduled').map(renderFolder)}
        </nav>
      ))}
      <div className={`sidebar-drawer-wrap ${moreFoldersOpen ? 'sidebar-drawer-wrap--expanded' : ''}`}>
        <button type="button" className={`sidebar-drawer-toggle ${moreFoldersOpen || secondaryFolderActive ? 'sidebar-drawer-toggle--active' : ''}`} aria-expanded={moreFoldersOpen} aria-controls="sidebar-more-folders" onClick={() => setMoreFoldersOpen(value => !value)}>
          <span>More</span><ChevronDown size={14} className="sidebar-drawer-toggle__chevron" />
        </button>
        <div className="sidebar-drawer-collapse">
          <div id="sidebar-more-folders" className="sidebar-drawer">
            <nav className="folder-nav" aria-label="More">
              {folders.filter(item => item.group === 'more' || item.label === 'Scheduled').map(renderFolder)}
            </nav>
          </div>
        </div>
      </div>
      <div className={`sidebar-drawer-wrap ${labelsOpen ? 'sidebar-drawer-wrap--expanded' : ''}`}>
        <div className={`sidebar-drawer-row ${labelsOpen || labelsActive ? 'sidebar-drawer-row--active' : ''}`}>
          <button type="button" className="sidebar-drawer-toggle" aria-expanded={labelsOpen} aria-controls="sidebar-labels" onClick={() => setLabelsOpen(value => !value)}>
            <span>Labels</span><ChevronDown size={14} className="sidebar-drawer-toggle__chevron" />
          </button>
          {labelsOpen && <button type="button" className="sidebar-drawer-toggle__settings" aria-label="Manage labels" onClick={() => setManageLabelsOpen(true)}><Settings2 size={14} /></button>}
        </div>
        <div className="sidebar-drawer-collapse">
          <div id="sidebar-labels" className="sidebar-drawer">
            <nav className="folder-nav" aria-label="Labels">
              {labels.map(({ name, color }) => (
                <button className={`folder-link ${query.toLowerCase() === `label:${name.toLowerCase()}` ? 'folder-link--active' : ''}`} key={name} onClick={() => goLabel(name)}>
                  <i className={`label-dot label-dot--${color}`} />{name}
                </button>
              ))}
            </nav>
          </div>
        </div>
      </div>
      <nav className="folder-nav sidebar-custom-folders" aria-label="Folders">
        <div className="nav-heading-row">
          <p className="nav-heading">Folders</p>
          <button type="button" aria-label="Manage folders" onClick={() => setManageFoldersOpen(true)}><Settings2 size={13} /></button>
        </div>
        {customFolders.map(customFolder => {
          const count = mailbox.filter(mail => mail.folder === customFolder.name).length
          const active = pathname.startsWith(`/mail/folders/${customFolder.id}`)
          return (
            <button key={customFolder.id} className={`folder-link ${active ? 'folder-link--active' : ''}`} aria-current={active ? 'page' : undefined} onClick={() => goCustomFolder(customFolder.id)}>
              <Folder size={17} /><span>{customFolder.name}</span>{count > 0 && <b>{count}</b>}
            </button>
          )
        })}
      </nav>
      <section className="sidebar-workspace">
        <p className="nav-heading">Workspace</p>
        <button className={`folder-link ${pathname.startsWith('/mail/calendar') ? 'folder-link--active' : ''}`} aria-current={pathname.startsWith('/mail/calendar') ? 'page' : undefined} onClick={() => { navigate('/mail/calendar'); setMoreFoldersOpen(false); setLabelsOpen(false); onCloseMobile() }}>
          <CalendarDays size={17} /><span>Calendar</span>
        </button>
        <button className={`folder-link ${pathname.startsWith('/mail/contacts') ? 'folder-link--active' : ''}`} aria-current={pathname.startsWith('/mail/contacts') ? 'page' : undefined} onClick={() => { navigate('/mail/contacts'); setMoreFoldersOpen(false); setLabelsOpen(false); onCloseMobile() }}>
          <UserRound size={17} /><span>Contacts</span>
        </button>
      </section>
      <div className="sidebar-footer">
        <div className="storage">
          <span>Storage</span>
          <strong>{storage.used} GB <small>/ 15 GB</small></strong>
          <div className="storage-bar"><span style={{ width: `${storage.percent}%` }} /></div>
        </div>
        <div className="profile-wrap">
          <button className="profile" onClick={event => { event.stopPropagation(); setProfileOpen(value => !value) }} aria-label="Open account menu" aria-haspopup="menu" aria-expanded={profileOpen}>
            <span className={`avatar avatar--${profile.color}`}>{profile.initials}</span>
            <span><strong>{profile.displayName}</strong><small>{profile.email}</small></span>
            <ChevronDown size={15} className={`profile-chevron ${profileOpen ? 'profile-chevron--open' : ''}`} />
          </button>
{profileOpen && (
            <div className="profile-menu" role="menu">
             <div className="profile-menu__account"><span className={`avatar avatar--${profile.color}`}>{profile.initials}</span><span><strong>{profile.displayName}</strong><small>{profile.email}</small></span><Check size={14} /></div>
               <div className="profile-menu__accounts" aria-label="Switch account">
                 <button type="button" className={`profile-menu__account-button ${activeAccount === unifiedViewId ? 'profile-menu__account-button--active' : ''}`} aria-label="Switch to unified inbox" onClick={() => switchAccount(unifiedViewId)}><Layers size={14} /><span>Unified inbox</span>{activeAccount === unifiedViewId && <Check size={13} />}</button>
                 {accounts.map(account => <button type="button" className={`profile-menu__account-button ${activeAccount === account.id ? 'profile-menu__account-button--active' : ''}`} aria-label={`Switch to ${account.email}`} onClick={() => switchAccount(account.id)} key={account.id}><span className={`avatar avatar--${account.color}`}>{account.initials}</span><span>{account.name}<small>{account.email}</small></span>{activeAccount === account.id && <Check size={13} />}</button>)}
                 <button type="button" className="profile-menu__manage" onClick={() => goAccount('/mail/settings?tab=accounts')}><Settings2 size={13} />Manage accounts</button>
               </div>
               <div className="profile-menu__divider" />
               <button type="button" role="menuitem" className="profile-menu__item" onClick={() => goAccount('/mail/settings')}><Settings2 size={14} />Settings</button>
              <button type="button" role="menuitem" className="profile-menu__item" onClick={() => goAccount('/mail/billing')}><CreditCard size={14} />Billing</button>
              <button type="button" role="menuitem" className="profile-menu__item" onClick={() => goAccount('/mail/admin')}><Server size={14} />Admin panel</button>
              <div className="profile-menu__divider" />
              <button type="button" role="menuitem" className="profile-menu__item" onClick={signOut}><LogOut size={14} />Sign out</button>
            </div>
          )}
        </div>
      </div>
      {manageLabelsOpen && <ManageLabels close={() => { setManageLabelsOpen(false); setLabelsOpen(true); setLabels(labelsApi.list()) }} />}
      {manageFoldersOpen && <ManageFolders close={() => { setManageFoldersOpen(false); setCustomFolders(foldersApi.list()) }} />}
    </aside>
  )
}
