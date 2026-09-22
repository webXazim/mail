import type { ReactNode } from 'react'
import { Link } from 'react-router-dom'
import { Moon } from 'lucide-react'
import { BrandIdentity } from './BrandIdentity'

type AuthShellProps = {
  eyebrow: string
  title: string
  copy: string
  children: ReactNode
  foot?: ReactNode
  asideIcon?: ReactNode
  asideCopy?: string
}

export function AuthShell({
  eyebrow,
  title,
  copy,
  children,
  foot,
  asideIcon = <Moon size={20} />,
  asideCopy = 'Keep important conversations moving without losing the details.',
}: AuthShellProps) {
  return (
    <main className="auth-page">
      <section className="auth-card">
        <div className="auth-brand">
          <BrandIdentity />
        </div>
        <p className="eyebrow">{eyebrow}</p>
        <h1>{title}</h1>
        <p className="auth-copy">{copy}</p>
        {children}
        {foot && <p className="auth-foot">{foot}</p>}
        <p className="auth-legal">
          By continuing, you agree to the <Link to="/legal/terms">Terms of Service</Link> and{' '}
          <Link to="/legal/privacy">Privacy Policy</Link>.
        </p>
      </section>
      <aside className="auth-aside">
        {asideIcon}
        <strong>Calm, focused email for teams.</strong>
        <span>{asideCopy}</span>
      </aside>
    </main>
  )
}
