import { useEffect, useMemo, useState } from 'react'
import { Check, CalendarDays, ChevronDown, CreditCard, Folder, LogOut, Server, Settings2, SquarePen, X } from 'lucide-react'
import { useLocation, useNavigate, useSearchParams } from 'react-router-dom'
import { folderFromPath, folderSlug, folders, getFolderCounts, navGroupTitles } from '../../lib/mail'
import { useMail } from '../../state/mail/MailContext'
import { draftsApi } from '../../services/drafts'
import { foldersApi } from '../../services/folders'
import { labelsApi } from '../../services/labels'
import { settingsApi } from '../../services/settings'
import { authApi } from '../../services/auth'
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
  const { mailbox, scheduledCount } = useMail()
  const folder = folderFromPath(pathname)
  const query = searchParams.get('q') || ''
  const counts = useMemo(() => getFolderCounts(mailbox), [mailbox])
  const profile = useMemo(() => {
    const displayName = settingsApi.load().displayName || 'Alex Morgan'
    const clear = displayName.trim().split(/\s+/)
    const initials = clear.map(part => part[0]?.toUpperCase() ?? '').slice(0, 2).join('') || 'AM'
    return { displayName, initials }
  }, [])
  const storage = useMemo(() => {
    const attachments = mailbox.filter(mail => mail.attachment).length
    const used = 0.4 + mailbox.filter(mail => mail.folder !== 'Trash').length * 0.002 + attachments * 0.012
    return { used: used.toFixed(1), percent: Math.min(100, Math.round((used / 15) * 100)) }
  }, [mailbox])
  const [resizing, setResizing] = useState(false)
  const [profileOpen, setProfileOpen] = useState(false)
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
  const goFolder = (label: Mailbox) => { navigate(`/mail/${folderSlug[label]}`); onCloseMobile() }
  const goLabel = (label: string) => {
    const target = folder in folderSlug ? folderSlug[folder as Mailbox] : 'inbox'
    navigate(`/mail/${target}?q=${encodeURIComponent(`label:${label}`)}`)
    onCloseMobile()
  }
  const goCustomFolder = (id: string) => { navigate(`/mail/folders/${id}`); onCloseMobile() }
  const closeMenu = () => setProfileOpen(false)
  const goAccount = (path: string) => { navigate(path); closeMenu(); onCloseMobile() }
  const signOut = () => { closeMenu(); void authApi.logout().finally(() => navigate('/login')) }
  const navGroups = ['mail', 'compose', 'more'] as const
  return (
    <aside id="sidebar" className={`sidebar ${mobile ? 'sidebar--open' : ''}`} aria-label="Mailbox navigation">
      <button className="sidebar-resizer" aria-label="Resize navigation" onMouseDown={() => setResizing(true)} />
      <div className="brand-row">
        <a className="brand" href="/"><span className="brand-mark">H</span><span>harbor<span>mail</span></span></a>
        <button className="icon-button sidebar-close" onClick={onCloseMobile} aria-label="Close navigation"><X size={17} /></button>
      </div>
      <button className="compose-button" onClick={onCompose}><SquarePen size={17} />Compose</button>
      <button className={`folder-link ${pathname.startsWith('/mail/calendar') ? 'folder-link--active' : ''}`} aria-current={pathname.startsWith('/mail/calendar') ? 'page' : undefined} onClick={() => { navigate('/mail/calendar'); onCloseMobile() }}>
        <CalendarDays size={17} /><span>Calendar</span>
      </button>
      {navGroups.map(group => (
        <nav key={group} className="folder-nav" aria-label={navGroupTitles[group]}>
          <p className="nav-heading">{navGroupTitles[group]}</p>
          {folders.filter(item => item.group === group).map(({ label, icon: Icon }) => {
            const count = label === 'Scheduled' ? scheduledCount : label === 'Drafts' ? (draftsApi.load() ? 1 : 0) : (counts[label] ?? 0)
            const active = folder === label
            return (
              <button key={label} className={`folder-link ${active ? 'folder-link--active' : ''}`} aria-current={active ? 'page' : undefined} onClick={() => goFolder(label)}>
                <Icon size={17} /><span>{label}</span>{count > 0 && <b>{count}</b>}
              </button>
            )
          })}
        </nav>
      ))}
      <section className="side-section">
        <div className="side-section__head"><p>Labels</p><button type="button" aria-label="Manage labels" onClick={() => setManageLabelsOpen(true)}><Settings2 size={13} /></button></div>
        {labels.map(({ name, color }) => (
          <button className={`label-link ${query.toLowerCase() === `label:${name.toLowerCase()}` ? 'label-link--active' : ''}`} key={name} onClick={() => goLabel(name)}>
            <i className={`label-dot label-dot--${color}`} />{name}
          </button>
        ))}
      </section>
      <section className="side-section">
        <div className="side-section__head"><p>Folders</p><button type="button" aria-label="Manage folders" onClick={() => setManageFoldersOpen(true)}><Settings2 size={13} /></button></div>
        {customFolders.map(customFolder => {
          const count = mailbox.filter(mail => mail.folder === customFolder.name).length
          const active = pathname.startsWith(`/mail/folders/${customFolder.id}`)
          return (
            <button key={customFolder.id} className={`folder-link ${active ? 'folder-link--active' : ''}`} aria-current={active ? 'page' : undefined} onClick={() => goCustomFolder(customFolder.id)}>
              <Folder size={17} /><span>{customFolder.name}</span>{count > 0 && <b>{count}</b>}
            </button>
          )
        })}
      </section>
      <div className="sidebar-footer">
        <div className="storage">
          <span>Storage</span>
          <strong>{storage.used} GB <small>/ 15 GB</small></strong>
          <div className="storage-bar"><span style={{ width: `${storage.percent}%` }} /></div>
        </div>
        <div className="profile-wrap">
          <button className="profile" onClick={event => { event.stopPropagation(); setProfileOpen(value => !value) }} aria-haspopup="menu" aria-expanded={profileOpen}>
            <span className="avatar avatar--teal">{profile.initials}</span>
            <span><strong>{profile.displayName}</strong><small>alex@harbor.co</small></span>
            <ChevronDown size={15} className={`profile-chevron ${profileOpen ? 'profile-chevron--open' : ''}`} />
          </button>
{profileOpen && (
            <div className="profile-menu" role="menu">
              <div className="profile-menu__account"><span className="avatar avatar--teal">{profile.initials}</span><span><strong>{profile.displayName}</strong><small>alex@harbor.co</small></span><Check size={14} /></div>
              <button type="button" role="menuitem" className="profile-menu__item" onClick={() => goAccount('/mail/settings')}><Settings2 size={14} />Settings</button>
              <button type="button" role="menuitem" className="profile-menu__item" onClick={() => goAccount('/mail/billing')}><CreditCard size={14} />Billing</button>
              <button type="button" role="menuitem" className="profile-menu__item" onClick={() => goAccount('/mail/admin')}><Server size={14} />Admin panel</button>
              <div className="profile-menu__divider" />
              <button type="button" role="menuitem" className="profile-menu__item" onClick={signOut}><LogOut size={14} />Sign out</button>
            </div>
          )}
        </div>
      </div>
      {manageLabelsOpen && <ManageLabels close={() => { setManageLabelsOpen(false); setLabels(labelsApi.list()) }} />}
      {manageFoldersOpen && <ManageFolders close={() => { setManageFoldersOpen(false); setCustomFolders(foldersApi.list()) }} />}
    </aside>
  )
}