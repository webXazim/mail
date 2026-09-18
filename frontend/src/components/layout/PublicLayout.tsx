import { useEffect, useState } from 'react'
import { Link, NavLink, Outlet } from 'react-router-dom'
import { ArrowRight, BarChart3, LockKeyhole, LifeBuoy, Mail } from 'lucide-react'
import { bootstrapSession, isSessionActive } from '../../lib/api'

export function PublicLayout() {
  const [signedIn, setSignedIn] = useState(isSessionActive())

  useEffect(() => {
    bootstrapSession().then(setSignedIn)
  }, [])

  return (
    <div className="site-shell">
      <header className="site-head">
        <Link to="/" className="site-brand">
          <span className="brand-mark">H</span>
          <strong>
            harbor<span>mail</span>
          </strong>
        </Link>
        <nav className="site-nav" aria-label="Primary">
          <NavLink
            to="/features"
            className={({ isActive }) => (isActive ? 'site-nav--active' : '')}
          >
            Features
          </NavLink>
          <NavLink
            to="/security"
            className={({ isActive }) => (isActive ? 'site-nav--active' : '')}
          >
            Security
          </NavLink>
          <NavLink to="/pricing" className={({ isActive }) => (isActive ? 'site-nav--active' : '')}>
            Pricing
          </NavLink>
          <NavLink to="/help" className={({ isActive }) => (isActive ? 'site-nav--active' : '')}>
            Help
          </NavLink>
        </nav>
        <div className="site-head__actions">
          {signedIn ? (
            <Link to="/mail/inbox" className="secondary-button">
              <Mail size={14} />
              Open app
            </Link>
          ) : (
            <Link to="/login" className="secondary-button">
              Sign in
            </Link>
          )}
        </div>
      </header>

      <main className="site-main">
        <Outlet />
      </main>

      <footer className="site-foot">
        <div className="site-foot__cols">
          <div className="site-foot__brand">
            <Link to="/" className="site-brand">
              <span className="brand-mark">H</span>
              <strong>
                harbor<span>mail</span>
              </strong>
            </Link>
            <p>Calm, focused email for teams. Space for the things that matter.</p>
            <Link to="/status" className="site-foot__status">
              <BarChart3 size={13} />
              All systems operational
            </Link>
          </div>
          <div className="site-foot__col">
            <p className="eyebrow">Product</p>
            <Link to="/features">Features</Link>
            <Link to="/security">Security</Link>
            <Link to="/pricing">Pricing</Link>
            <Link to="/mail/inbox">Sign in</Link>
          </div>
          <div className="site-foot__col">
            <p className="eyebrow">Legal</p>
            <Link to="/legal/terms">Terms of Service</Link>
            <Link to="/legal/privacy">Privacy Policy</Link>
            <Link to="/legal/aup">Acceptable Use</Link>
            <Link to="/legal/security">Security &amp; Compliance</Link>
          </div>
          <div className="site-foot__col">
            <p className="eyebrow">Company</p>
            <a href="mailto:sales@harbor.co">
              <LockKeyhole size={13} />
              Contact sales
            </a>
            <Link to="/help">
              <LifeBuoy size={13} />
              Help center
            </Link>
            <Link to="/contact">Contact support</Link>
            <Link to="/status">Service status</Link>
          </div>
        </div>
        <div className="site-foot__bottom">
          <span>© 2026 Harbor Mail, Inc.</span>
          <span>
            Made in San Francisco <ArrowRight size={12} />
          </span>
        </div>
      </footer>
    </div>
  )
}
