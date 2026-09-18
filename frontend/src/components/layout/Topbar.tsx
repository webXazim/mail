import { useEffect, useRef } from 'react'
import { ArrowLeft, CircleHelp, Menu, Search, X } from 'lucide-react'
import { useLocation, useNavigate, useSearchParams } from 'react-router-dom'
import { useIsMobile } from '../../hooks/useIsMobile'

type TopbarProps = { onOpenMobile: () => void; mobileOpen: boolean; onHelp: () => void }

export function Topbar({ onOpenMobile, mobileOpen, onHelp }: TopbarProps) {
  const navigate = useNavigate()
  const { pathname } = useLocation()
  const [searchParams] = useSearchParams()
  const query = searchParams.get('q') || ''
  const isMobile = useIsMobile()
  const inputRef = useRef<HTMLInputElement>(null)
  const onSearchScreen = pathname.startsWith('/mail/search')
  useEffect(() => {
    if (!isMobile || !onSearchScreen) return
    const node = inputRef.current
    if (node) {
      node.focus()
      node.select()
    }
  }, [isMobile, onSearchScreen])
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
  const openSearch = () => {
    if (onSearchScreen) return
    navigate(query ? `/mail/search?q=${encodeURIComponent(query)}` : '/mail/search', {
      replace: true,
    })
  }
  const searchField = (
    <>
      {!(isMobile && onSearchScreen) && <Search size={17} />}
      <input
        ref={inputRef}
        value={query}
        onFocus={openSearch}
        onChange={(event) =>
          navigate(
            event.target.value
              ? `/mail/search?q=${encodeURIComponent(event.target.value)}`
              : '/mail/search',
            { replace: true },
          )
        }
        placeholder="Search mail"
        aria-label="Search mail"
      />
      {isMobile && onSearchScreen && query && (
        <button
          type="button"
          className="search-clear"
          aria-label="Clear search"
          onClick={(event) => {
            event.preventDefault()
            event.stopPropagation()
            navigate('/mail/search')
          }}
        >
          <X size={17} />
        </button>
      )}
      <kbd aria-hidden="true">/</kbd>
    </>
  )
  if (isMobile && onSearchScreen) {
    return (
      <header className="topbar topbar--searching">
        <button
          type="button"
          className="icon-button"
          aria-label="Back to inbox"
          onClick={() => navigate('/mail/inbox')}
        >
          <ArrowLeft size={18} />
        </button>
        <label className="search search--grown" onClick={openSearch}>
          {searchField}
        </label>
      </header>
    )
  }
  return (
    <header className={`topbar${onSearchScreen ? ' topbar--searching' : ''}`}>
      <button
        className="icon-button mobile-menu"
        onClick={onOpenMobile}
        aria-label={mobileOpen ? 'Close navigation' : 'Open navigation'}
        aria-expanded={mobileOpen}
        aria-controls="sidebar"
      >
        <Menu size={18} />
      </button>
      <label className={`search${onSearchScreen ? ' search--grown' : ''}`} onClick={openSearch}>
        {searchField}
      </label>
      <div className="top-actions">
        <button className="icon-button" aria-label="Keyboard shortcuts" onClick={onHelp}>
          <CircleHelp size={17} />
        </button>
        <span className="top-divider" />
        <span className="avatar avatar--teal">AM</span>
      </div>
    </header>
  )
}
