import { useEffect, useState, type FormEvent } from 'react'
import { useNavigate } from 'react-router-dom'
import { ArrowLeft, Check, CreditCard, Download, FileText } from 'lucide-react'
import { plans } from '../lib/plans'

const invoices = [
  { id: 'INV-2026-036', date: 'September 1, 2026', amount: '$80.00' },
  { id: 'INV-2026-033', date: 'August 1, 2026', amount: '$80.00' },
  { id: 'INV-2026-030', date: 'July 1, 2026', amount: '$80.00' },
]

type PaymentMethod = { brand: string; last4: string; expiry: string }

const paymentFile = (invoice: { id: string; date: string; amount: string }) =>
  `Harbor Mail invoice\n==================\nInvoice: ${invoice.id}\nDate: ${invoice.date}\nAmount: ${invoice.amount}\nStatus: Paid\n`

const downloadFile = (name: string, content: string) => {
  const blob = new Blob([content], { type: 'text/plain' })
  const url = URL.createObjectURL(blob)
  const link = document.createElement('a')
  link.href = url
  link.download = name
  link.click()
  URL.revokeObjectURL(url)
}

export function BillingPage() {
  const navigate = useNavigate()
  const [tab, setTab] = useState<'plan' | 'invoices'>('plan')
  const [plan, setPlan] = useState('team')
  const [changing, setChanging] = useState(false)
  const [planNotice, setPlanNotice] = useState('')
  const [downloaded, setDownloaded] = useState<string[]>([])
  const [payment, setPayment] = useState<PaymentMethod>({ brand: 'Visa', last4: '4242', expiry: '09 / 2028' })
  const [editingPayment, setEditingPayment] = useState(false)
  const [paymentForm, setPaymentForm] = useState({ number: '', expiry: '', cvc: '' })
  const [paymentNotice, setPaymentNotice] = useState('')

  const back = () => navigate('/mail/inbox')

  const current = plans.find(item => item.id === plan) ?? plans[1]

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') navigate('/mail/inbox')
    }
    window.addEventListener('keydown', handleKeyDown)
    return () => window.removeEventListener('keydown', handleKeyDown)
  }, [navigate])

  const applyPlan = () => {
    const next = plans.find(item => item.id === plan)
    if (next) setPlanNotice(`Your plan is now ${next.name}.`)
    setChanging(false)
    window.setTimeout(() => setPlanNotice(''), 4000)
  }

  const downloadInvoice = (id: string) => {
    const invoice = invoices.find(item => item.id === id)
    if (invoice) downloadFile(`${invoice.id}.txt`, paymentFile(invoice))
    setDownloaded(current => [...current, id])
    window.setTimeout(() => setDownloaded(current => current.filter(item => item !== id)), 3000)
  }

  const startPaymentEdit = () => { setEditingPayment(true); setPaymentNotice('') }
  const cancelPaymentEdit = () => { setEditingPayment(false); setPaymentForm({ number: '', expiry: '', cvc: '' }) }
  const savePayment = (event: FormEvent) => {
    event.preventDefault()
    const number = paymentForm.number.replace(/\D/g, '')
    const expiry = paymentForm.expiry.trim()
    if (number.length < 13 || number.length > 19) { setPaymentNotice('Enter a valid card number'); return }
    if (!/^(0[1-9]|1[0-2])\s*\/\s*\d{2}$/.test(expiry)) { setPaymentNotice('Enter a valid expiry as MM / YY'); return }
    if (paymentForm.cvc.replace(/\D/g, '').length < 3) { setPaymentNotice('Enter the 3-digit security code'); return }
    setPayment({ brand: number.startsWith('4') ? 'Visa' : number.startsWith('5') ? 'Mastercard' : 'Card', last4: number.slice(-4), expiry: `${expiry.slice(0, 2)} / ${expiry.slice(-2)}` })
    setEditingPayment(false)
    setPaymentForm({ number: '', expiry: '', cvc: '' })
    setPaymentNotice('Payment method updated')
    window.setTimeout(() => setPaymentNotice(''), 4000)
  }

  return (
    <div className="settings-page" role="region" aria-label="Billing">
      <header className="calendar-head">
        <div>
          <p className="eyebrow">Harbor Mail</p>
          <h1>Billing</h1>
        </div>
        <div className="calendar-head__actions">
          <button type="button" className="secondary-button" onClick={() => navigate('/mail/pricing')}>Compare plans</button>
          <button type="button" className="secondary-button" onClick={back}><ArrowLeft size={14} />Back to inbox</button>
        </div>
      </header>

      <nav className="admin-nav" aria-label="Billing sections">
        <button type="button" className={tab === 'plan' ? 'admin-nav--active' : ''} aria-current={tab === 'plan' ? 'page' : undefined} onClick={() => setTab('plan')}><CreditCard size={14} />Plan</button>
        <button type="button" className={tab === 'invoices' ? 'admin-nav--active' : ''} aria-current={tab === 'invoices' ? 'page' : undefined} onClick={() => setTab('invoices')}><FileText size={14} />Invoices</button>
      </nav>

      {tab === 'plan' && (
        <>
          <div className="settings-section">
            <h3>Current plan</h3>
            <div className="billing-plan">
              <div><strong>{current.name}</strong><small><span>{current.price} per seat / month</span> · Renews October 1, 2026</small></div>
              <button type="button" className="secondary-button" onClick={() => setChanging(value => !value)}>{changing ? 'Cancel' : 'Change plan'}</button>
            </div>
            {planNotice && <p className="settings-notice settings-notice--ok">{planNotice}</p>}
            <div className="storage"><span>Seats used</span><strong>1 <small>/ {current.seats}</small></strong><div className="storage-bar"><span style={{ width: `${(1 / current.seats) * 100}%` }} /></div></div>
          </div>
          {changing && (
            <div className="settings-section">
              <h3>Choose a plan</h3>
              {plans.map(item => (
                <button type="button" className={`plan-option ${plan === item.id ? 'plan-option--active' : ''}`} key={item.id} onClick={() => setPlan(item.id)} aria-pressed={plan === item.id}>
                  <span><strong>{item.name}</strong><small>{item.detail}</small></span>
                  <span className="plan-option__price"><strong>{item.price}</strong><small>per seat / month</small></span>
                  {plan === item.id && <Check size={15} />}
                </button>
              ))}
              <div className="row-actions">
                <button type="button" className="secondary-button" onClick={() => navigate('/mail/pricing')}>Compare all plans</button>
                <button type="button" className="primary-button" onClick={applyPlan}>Apply plan</button>
              </div>
            </div>
          )}
          <div className="settings-section">
            <h3>Payment method</h3>
            {editingPayment ? (
              <form className="payment-form" onSubmit={savePayment}>
                <label>Card number<input inputMode="numeric" autoComplete="cc-number" value={paymentForm.number} onChange={event => setPaymentForm(current => ({ ...current, number: event.target.value }))} placeholder="4242 4242 4242 4242" /></label>
                <div className="payment-form__row">
                  <label>Expiry (MM / YY)<input inputMode="numeric" autoComplete="cc-exp" value={paymentForm.expiry} onChange={event => setPaymentForm(current => ({ ...current, expiry: event.target.value }))} placeholder="09 / 28" /></label>
                  <label>Security code<input inputMode="numeric" autoComplete="cc-csc" type="password" value={paymentForm.cvc} onChange={event => setPaymentForm(current => ({ ...current, cvc: event.target.value }))} placeholder="123" /></label>
                </div>
                {paymentNotice && <p className={`settings-notice ${paymentNotice === 'Payment method updated' ? 'settings-notice--ok' : ''}`}>{paymentNotice}</p>}
                <div className="row-actions">
                  <button type="button" className="secondary-button" onClick={cancelPaymentEdit}>Cancel</button>
                  <button className="primary-button">Save card</button>
                </div>
              </form>
            ) : (
              <div className="billing-plan">
                <div><strong>{payment.brand} ending in {payment.last4}</strong><small>Expires {payment.expiry}</small></div>
                <button type="button" className="secondary-button" onClick={startPaymentEdit}>Update</button>
                {paymentNotice && <small className="settings-notice settings-notice--ok">{paymentNotice}</small>}
              </div>
            )}
          </div>
          <footer>
            <button type="button" className="primary-button" onClick={back}>Done</button>
          </footer>
        </>
      )}

      {tab === 'invoices' && (
        <div className="settings-section">
          <h3>Invoices</h3>
          {invoices.map(invoice => (
            <div className="billing-row" key={invoice.id}>
              <div><strong>{invoice.id}</strong><small>{invoice.date} · {invoice.amount}</small></div>
              <span className="billing-paid"><Check size={13} />Paid</span>
              {downloaded.includes(invoice.id) ? <small className="settings-notice settings-notice--ok">Sent to your inbox</small> : <button type="button" className="secondary-button" onClick={() => downloadInvoice(invoice.id)}><Download size={13} />Download</button>}
            </div>
          ))}
          <footer>
            <button type="button" className="primary-button" onClick={back}>Done</button>
          </footer>
        </div>
      )}
    </div>
  )
}