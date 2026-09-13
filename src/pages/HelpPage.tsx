import { useState } from 'react'
import { Link } from 'react-router-dom'
import { ArrowRight, ChevronDown, LifeBuoy, Search } from 'lucide-react'
import { faqs, filterHelp, type HelpCategory } from '../lib/help'

const categories: HelpCategory[] = ['Getting started', 'Mail & inbox', 'Security', 'Billing']

export function HelpPage() {
  const [query, setQuery] = useState('')
  const { articles, faqs: visibleFaqs } = filterHelp(query)
  const searching = Boolean(query.trim())

  return (
    <div className="site-page">
      <section className="site-page__head site-page__head--center">
        <p className="eyebrow">Help center</p>
        <h1>
          How can <span>we help?</span>
        </h1>
        <p className="site-page__intro">
          Guides for the inbox, security, and billing. Search an article or jump straight to the
          FAQ.
        </p>
        <label className="help-search">
          <Search size={17} />
          <input
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder="Search articles and FAQs"
            aria-label="Search help articles"
          />
        </label>
      </section>

      {searching && (
        <p className="settings-hint" role="status">
          {articles.length + visibleFaqs.length} result
          {articles.length + visibleFaqs.length === 1 ? '' : 's'} for &ldquo;{query}&rdquo;
        </p>
      )}

      {categories.map((category) => {
        const grouped = articles.filter((article) => article.category === category)
        if (grouped.length === 0) return null
        return (
          <section className="feature-group" key={category}>
            <header className="feature-group__head">
              <span className="feature-group__icon">
                <LifeBuoy size={16} />
              </span>
              <h2>{category}</h2>
            </header>
            <div className="feature-grid">
              {grouped.map((article) => (
                <article className="feature-card" key={article.title}>
                  <h3>{article.title}</h3>
                  <p>{article.summary}</p>
                </article>
              ))}
            </div>
          </section>
        )
      })}

      <section className="site-section">
        <header className="feature-group__head">
          <span className="feature-group__icon">
            <LifeBuoy size={16} />
          </span>
          <h2>Frequently asked questions</h2>
        </header>
        <div className="faq-list">
          {(searching ? visibleFaqs : faqs).map((faq) => (
            <details className="faq-item" key={faq.q}>
              <summary>
                {faq.q}
                <ChevronDown size={15} />
              </summary>
              <p>{faq.a}</p>
            </details>
          ))}
          {(searching ? visibleFaqs : faqs).length === 0 && (
            <p className="settings-hint">No FAQs match that search. Try a different keyword.</p>
          )}
        </div>
      </section>

      <section className="home-band">
        <div>
          <p className="eyebrow">Still stuck?</p>
          <h2>
            Talk to a{' '}
            <span>
              <LifeBuoy size={15} />
              human
            </span>{' '}
            on our team.
          </h2>
        </div>
        <Link to="/contact" className="secondary-button">
          Contact support <ArrowRight size={14} />
        </Link>
      </section>

      {searching && articles.length === 0 && visibleFaqs.length === 0 && (
        <p className="settings-hint">No articles or FAQs match that search.</p>
      )}
    </div>
  )
}
