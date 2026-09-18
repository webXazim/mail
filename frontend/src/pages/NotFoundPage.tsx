import { useEffect } from 'react'
import { useNavigate } from 'react-router-dom'
import { ArrowLeft, Settings2 } from 'lucide-react'

export function NotFoundPage({ full = false }: { full?: boolean }) {
  const navigate = useNavigate()

  useEffect(() => {
    const handle = (event: KeyboardEvent) => {
      if (event.key === 'Escape') navigate('/mail/inbox')
    }
    window.addEventListener('keydown', handle)
    return () => window.removeEventListener('keydown', handle)
  }, [navigate])

  if (full)
    return (
      <main>
        <div className="not-found not-found--full" role="region" aria-label="Page not found">
          <p className="not-found__code">404</p>
          <h1>Nothing here but empty servers</h1>
          <p className="settings-hint">This page doesn&rsquo;t exist or has been moved.</p>
          <div className="not-found__actions">
            <button
              type="button"
              className="primary-button"
              onClick={() => navigate('/mail/inbox')}
            >
              <ArrowLeft size={15} />
              Back to inbox
            </button>
            <button
              type="button"
              className="secondary-button"
              onClick={() => navigate('/mail/settings')}
            >
              <Settings2 size={15} />
              Settings
            </button>
          </div>
        </div>
      </main>
    )

  return (
    <div className="not-found" role="region" aria-label="Page not found">
      <p className="not-found__code">404</p>
      <h1>Nothing here but empty servers</h1>
      <p className="settings-hint">This page doesn&rsquo;t exist or has been moved.</p>
      <div className="not-found__actions">
        <button type="button" className="primary-button" onClick={() => navigate('/mail/inbox')}>
          <ArrowLeft size={15} />
          Back to inbox
        </button>
        <button
          type="button"
          className="secondary-button"
          onClick={() => navigate('/mail/settings')}
        >
          <Settings2 size={15} />
          Settings
        </button>
      </div>
    </div>
  )
}
