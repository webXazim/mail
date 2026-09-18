import { Link, Navigate, useLocation } from 'react-router-dom'
import { CalendarDays } from 'lucide-react'
import { legalDocs, legalNav, type LegalDoc } from '../lib/legal'

function LegalPageInner({ doc }: { doc: LegalDoc }) {
  const { hash } = useLocation()
  const active = hash ? hash.slice(1) : ''

  return (
    <main className="legal-page">
      <header className="legal-head">
        <Link to="/login" className="legal-brand">
          <span className="brand-mark">H</span>
          <strong>
            harbor<span>mail</span>
          </strong>
        </Link>
        <nav className="legal-nav" aria-label="Legal documents">
          {legalNav.map((item) => (
            <Link
              key={item.id}
              to={`/legal/${item.id}`}
              className={item.id === doc.id ? 'legal-nav--active' : ''}
            >
              {item.label}
            </Link>
          ))}
        </nav>
      </header>

      <div className="legal-body">
        <aside className="legal-toc" aria-label="On this page">
          <p className="eyebrow">On this page</p>
          <ul>
            {doc.sections.map((section) => (
              <li key={section.id}>
                <a
                  href={`#${section.id}`}
                  className={active === section.id ? 'legal-toc--active' : ''}
                >
                  {section.heading.replace(/^\d+\.\s*/, '')}
                </a>
              </li>
            ))}
          </ul>
        </aside>

        <article className="legal-content">
          <p className="eyebrow">Harbor Mail</p>
          <h1>{doc.title}</h1>
          <p className="legal-updated">
            <CalendarDays size={13} /> Last updated {doc.updated}
          </p>
          <p className="legal-intro">{doc.intro}</p>
          {doc.sections.map((section) => (
            <section key={section.id} id={section.id} className="legal-section">
              <h2>{section.heading}</h2>
              {section.body.map((paragraph) => (
                <p key={paragraph}>{paragraph}</p>
              ))}
            </section>
          ))}
        </article>
      </div>

      <footer className="legal-foot">
        <span>
          Questions? <a href="mailto:legal@harbor.co">legal@harbor.co</a>
        </span>
        <span>© 2026 Harbor Mail, Inc.</span>
      </footer>
    </main>
  )
}

export function LegalPage({ docId }: { docId: string }) {
  const doc = legalDocs[docId]
  if (!doc) return <Navigate to="/legal/terms" replace />
  return <LegalPageInner doc={doc} />
}
