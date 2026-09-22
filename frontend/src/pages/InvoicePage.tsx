import { useEffect, useState } from 'react'
import { useNavigate, useParams } from 'react-router-dom'
import { ArrowLeft, FileText, Printer } from 'lucide-react'
import { invoiceFor, type Invoice } from '../lib/invoices'
import { billingApi, invoiceFromOrder } from '../services/billing'

export function InvoicePage() {
  const navigate = useNavigate()
  const { invoiceId } = useParams()
  const [invoice, setInvoice] = useState<Invoice | undefined>(() => invoiceFor(invoiceId ?? ''))
  const [loading, setLoading] = useState(!invoice)

  useEffect(() => {
    let alive = true
    if (!invoice && invoiceId) {
      void billingApi.invoices().then((orders) => {
        if (!alive) return
        const found = orders.find((order) => order.invoice_number?.toLowerCase() === invoiceId.toLowerCase())
        setInvoice(found ? invoiceFromOrder(found) ?? undefined : undefined)
        setLoading(false)
      }).catch(() => alive && setLoading(false))
    }
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') navigate('/mail/billing')
    }
    window.addEventListener('keydown', handleKeyDown)
    return () => { alive = false; window.removeEventListener('keydown', handleKeyDown) }
  }, [invoice, invoiceId, navigate])

  if (loading) return <div className="route-loader"><div className="loading-spinner" /></div>

  if (!invoice) {
    return (
      <div className="settings-page" role="region" aria-label="Invoice">
        <header className="calendar-head"><div><p className="eyebrow">CS Mail / Billing</p><h1>Invoice not found</h1></div></header>
        <div className="list-state"><FileText size={26} /><strong>We couldn&apos;t find that invoice</strong><span>Check the invoice number or return to Billing.</span><button type="button" className="primary-button" onClick={() => navigate('/mail/billing')}>Back to billing</button></div>
      </div>
    )
  }

  const statusLabel = invoice.status === 'paid' ? 'Paid' : invoice.status === 'void' ? 'Void' : 'Payment due'
  return (
    <div className="settings-page invoice-print-page" role="region" aria-label={`Invoice ${invoice.id}`}>
      <header className="calendar-head no-print">
        <div><p className="eyebrow">CS Mail / Billing</p><h1>{invoice.id}</h1></div>
        <div className="calendar-head__actions">
          <button type="button" className="secondary-button" onClick={() => window.print()}><Printer size={14} />Print / save PDF</button>
          <button type="button" className="secondary-button" onClick={() => navigate('/mail/billing')}><ArrowLeft size={14} />Back to billing</button>
        </div>
      </header>

      <section className="settings-section invoice-sheet">
        <div className="admin-section-head"><div><p className="eyebrow">CS Mail</p><h2>Invoice {invoice.id}</h2></div><div className={`receipt-status receipt-status--${invoice.status}`}>{statusLabel}</div></div>
        <div className="receipt-grid">
          <div className="receipt-block"><span className="receipt-label">Seller</span><strong>{invoice.sellerName}</strong>{invoice.sellerCr && <small>CR {invoice.sellerCr}</small>}{invoice.sellerVat && <small>VAT {invoice.sellerVat}</small>}<small>{invoice.sellerAddress}</small></div>
          <div className="receipt-block"><span className="receipt-label">Billed to</span><strong>{invoice.buyerName}</strong>{invoice.buyerCr && <small>CR {invoice.buyerCr}</small>}{invoice.buyerVat && <small>VAT {invoice.buyerVat}</small>}<small>{invoice.buyerAddress}</small><small>{invoice.billedTo}</small></div>
          <div className="receipt-block"><span className="receipt-label">Issue / due</span><strong>{invoice.date}</strong><small>Due {invoice.dueDate}</small><small>Period {invoice.period}</small></div>
          <div className="receipt-block"><span className="receipt-label">Total</span><strong className="receipt-amount">{invoice.total}</strong><small>{statusLabel}</small><small>{invoice.paymentMethod}</small></div>
        </div>
      </section>

      <section className="settings-section invoice-sheet">
        <h2>Invoice items</h2>
        <div className="billing-row"><div><strong>{invoice.plan}</strong><small>{invoice.includedMailboxCount} mailbox{invoice.includedMailboxCount === 1 ? '' : 'es'} included · annual base subscription</small></div><span className="receipt-line-amount">{invoice.baseRate}</span></div>
        {invoice.extraMailboxCount > 0 && <div className="billing-row"><div><strong>Additional mailboxes</strong><small>{invoice.extraMailboxCount} × {invoice.extraMailboxUnitPrice} · {invoice.mailboxCount} total mailboxes</small></div><span className="receipt-line-amount">{invoice.extraMailboxTotal}</span></div>}
        <div className="billing-row billing-row--quiet"><div><strong>Subtotal</strong></div><span className="receipt-line-amount">{invoice.subtotal}</span></div>
        <div className="billing-row billing-row--quiet"><div><strong>Tax</strong><small>{invoice.taxRate}</small></div><span className="receipt-line-amount">{invoice.tax}</span></div>
        <div className="receipt-total"><span>Total</span><strong>{invoice.total}</strong></div>
        {invoice.status === 'issued' && <p className="settings-hint">This invoice is awaiting manual payment. After paying, submit the bank/payment reference from Billing for verification.</p>}
      </section>

      <footer className="no-print"><button type="button" className="secondary-button" onClick={() => navigate('/mail/billing')}>Back to billing</button><button type="button" className="primary-button" onClick={() => window.print()}><Printer size={14} />Print / save PDF</button></footer>
    </div>
  )
}
