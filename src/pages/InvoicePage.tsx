import { useEffect, useState } from 'react'
import { useNavigate, useParams } from 'react-router-dom'
import { ArrowLeft, Check, Download, FileText } from 'lucide-react'
import { invoiceFor, invoiceToText } from '../lib/invoices'
import { downloadTextFile } from '../lib/files'

export function InvoicePage() {
  const navigate = useNavigate()
  const { invoiceId } = useParams()
  const invoice = invoiceFor(invoiceId ?? '')
  const [sent, setSent] = useState(false)

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') navigate('/mail/billing')
    }
    window.addEventListener('keydown', handleKeyDown)
    return () => window.removeEventListener('keydown', handleKeyDown)
  }, [navigate])

  if (!invoice) {
    return (
      <div className="settings-page" role="region" aria-label="Invoice">
        <header className="calendar-head">
          <div>
            <p className="eyebrow">Harbor Mail / Billing</p>
            <h1>Invoice not found</h1>
          </div>
          <div className="calendar-head__actions">
            <button
              type="button"
              className="secondary-button"
              onClick={() => navigate('/mail/billing')}
            >
              <ArrowLeft size={14} />
              Back to billing
            </button>
          </div>
        </header>
        <div className="list-state">
          <FileText size={26} />
          <strong>We couldn&apos;t find that invoice</strong>
          <span>Check the link and try again.</span>
          <button
            type="button"
            className="primary-button"
            onClick={() => navigate('/mail/billing')}
          >
            Back to billing
          </button>
        </div>
      </div>
    )
  }

  const download = () => {
    downloadTextFile(`${invoice.id}.txt`, invoiceToText(invoice))
    setSent(true)
    window.setTimeout(() => setSent(false), 3000)
  }

  return (
    <div className="settings-page" role="region" aria-label={`Receipt ${invoice.id}`}>
      <header className="calendar-head">
        <div>
          <p className="eyebrow">Harbor Mail / Billing</p>
          <h1>{invoice.id}</h1>
        </div>
        <div className="calendar-head__actions">
          {sent ? (
            <small className="settings-notice settings-notice--ok">
              <Check size={13} />
              Sent to your inbox
            </small>
          ) : (
            <button type="button" className="secondary-button" onClick={download}>
              <Download size={14} />
              Download receipt
            </button>
          )}
          <button
            type="button"
            className="secondary-button"
            onClick={() => navigate('/mail/billing')}
          >
            <ArrowLeft size={14} />
            Back to billing
          </button>
        </div>
      </header>

      <section className="settings-section">
        <div className={`receipt-status receipt-status--${invoice.status}`}>
          <Check size={13} />
          {invoice.status === 'paid' ? 'Paid' : 'Pending'}
        </div>
        <div className="receipt-grid">
          <div className="receipt-block">
            <span className="receipt-label">Billed to</span>
            <strong>{invoice.billedTo}</strong>
            <small>harbor-mail · Harbor Team</small>
          </div>
          <div className="receipt-block">
            <span className="receipt-label">Billing period</span>
            <strong>{invoice.period}</strong>
            <small>Issued {invoice.date}</small>
          </div>
          <div className="receipt-block">
            <span className="receipt-label">Payment method</span>
            <strong>{invoice.paymentMethod}</strong>
            <small>Charged on issue date</small>
          </div>
          <div className="receipt-block">
            <span className="receipt-label">Amount paid</span>
            <strong className="receipt-amount">{invoice.total}</strong>
            <small>Status: {invoice.status === 'paid' ? 'Paid' : 'Pending'}</small>
          </div>
        </div>
      </section>

      <section className="settings-section">
        <h2>Receipt</h2>
        <div className="billing-row">
          <div>
            <strong>{invoice.plan}</strong>
            <small>
              {invoice.seats} seat · {invoice.rate} per seat / month
            </small>
          </div>
          <span className="receipt-line-amount">{invoice.rate}</span>
        </div>
        <div className="billing-row billing-row--quiet">
          <div>
            <strong>Subtotal</strong>
            <small>Harbor Team · {invoice.seats} seat</small>
          </div>
          <span className="receipt-line-amount">{invoice.subtotal}</span>
        </div>
        <div className="billing-row billing-row--quiet">
          <div>
            <strong>Tax</strong>
            <small>No tax applied</small>
          </div>
          <span className="receipt-line-amount">{invoice.tax}</span>
        </div>
        <div className="receipt-total">
          <span>Total due</span>
          <strong>{invoice.total}</strong>
        </div>
        <p className="settings-hint">
          Next invoice for {invoice.period === 'September 2026' ? 'October 2026' : invoice.period}{' '}
          will be issued on the first of the month.
        </p>
      </section>

      <footer>
        <button
          type="button"
          className="secondary-button"
          onClick={() => navigate('/mail/billing')}
        >
          Back to billing
        </button>
        <button type="button" className="primary-button" onClick={download}>
          Download receipt
        </button>
      </footer>
    </div>
  )
}
