import { useEffect, useRef } from 'react'
import { Bell, CircleHelp, Menu, Search } from 'lucide-react'
import { useLocation, useNavigate, useSearchParams } from 'react-router-dom'
import { folderFromPath, folderPath } from '../../lib/mail'

type TopbarProps = { onOpenMobile: () => void; mobileOpen: boolean; onHelp: () => void }

export function Topbar({ onOpenMobile, mobileOpen, onHelp }: TopbarProps) {
  const navigate = useNavigate()
  const { pathname } = useLocation()
  const [searchParams] = useSearchParams()
  const folder = folderFromPath(pathname)
  const query = searchParams.get('q') || ''
  const inputRef = useRef<HTMLInputElement>(null)
  useEffect(() => {
    const handle = (event: globalThis.KeyboardEvent) => {
      if (event.key !== '/') return
      const target = event.target as HTMLElement
      if (target.tagName === 'INPUT' || target.tagName === 'TEXTAREA') return
      event.preventDefault()
      inputRef.current?.focus()
    }
    window.addEventListener('keydown', handle)
    return () => window.removeEventListener('keydown', handle)
  }, [])
  const runSearch = (value: string) => {
    navigate(value ? `/mail/${folderPath(folder)}?q=${encodeURIComponent(value)}` : `/mail/${folderPath(folder)}`, { replace: true })
  }
  return (
    <header className="topbar">
      <button className="icon-button mobile-menu" onClick={onOpenMobile} aria-label={mobileOpen ? 'Close navigation' : 'Open navigation'} aria-expanded={mobileOpen} aria-controls="sidebar"><Menu size={18} /></button>
      <label className="search">
        <Search size={17} />
        <input ref={inputRef} value={query} onChange={event => runSearch(event.target.value)} placeholder="Search mail" aria-label="Search mail" />
        <kbd aria-hidden="true">/</kbd>
      </label>
      <div className="top-actions">
        <button className="icon-button" aria-label="Keyboard shortcuts" onClick={onHelp}><CircleHelp size={17} /></button>
        <button className="icon-button" aria-label="Notifications" onClick={() => navigate('/mail/notifications')}><Bell size={17} /></button>
        <span className="top-divider" />
        <span className="avatar avatar--teal">AM</span>
      </div>
    </header>
  )
}