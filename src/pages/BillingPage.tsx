import { useEffect, useState, type FormEvent } from 'react'
import { useNavigate } from 'react-router-dom'
import { ArrowLeft, Check, CreditCard, Download, FileText, Plus, Trash2, Star } from 'lucide-react'
import { plans } from '../lib/plans'
import { invoices, invoiceToText } from '../lib/invoices'
import { downloadTextFile } from '../lib/files'
import { auditApi } from '../services/audit'

type PaymentMethod = { id: string; brand: string; last4: string; expiry: string; default: boolean }

export function BillingPage() {
  const navigate = useNavigate()
  const [tab, setTab] = useState<'plan' | 'invoices'>('plan')
  const [plan, setPlan] = useState('team')
  const [changing, setChanging] = useState(false)
  const [planNotice, setPlanNotice] = useState('')
  const [downloaded, setDownloaded] = useState<string[]>([])
  const [methods, setMethods] = useState<PaymentMethod[]>([
    { id: 'pm-visa', brand: 'Visa', last4: '4049', expiry: '09 / 2028', default: true },
    { id: 'pm-mc', brand: 'Mastercard', last4: '1853', expiry: '03 / 2027', default: false },
  ])
  const [addingPayment, setAddingPayment] = useState(false)
  const [paymentForm, setPaymentForm] = useState({ number: '', expiry: '', cvc: '' })
  const [paymentNotice, setPaymentNotice] = useState('')

  const back = () => navigate('/mail/inbox')

  const current = plans.find((item) => item.id === plan) ?? plans[1]

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') navigate('/mail/inbox')
    }
    window.addEventListener('keydown', handleKeyDown)
    return () => window.removeEventListener('keydown', handleKeyDown)
  }, [navigate])

  const applyPlan = () => {
    const next = plans.find((item) => item.id === plan)
    if (next) {
      setPlanNotice(`Your plan is now ${next.name}.`)
      auditApi.add('billing', 'Plan changed', `Switched to ${next.name}`)
    }
    setChanging(false)
    window.setTimeout(() => setPlanNotice(''), 4000)
  }

  const downloadInvoice = (id: string) => {
    const invoice = invoices.find((item) => item.id === id)
    if (invoice) downloadTextFile(`${invoice.id}.txt`, invoiceToText(invoice))
    setDownloaded((current) => [...current, id])
    window.setTimeout(() => setDownloaded((current) => current.filter((item) => item !== id)), 3000)
  }

  const resetPaymentForm = () => {
    setAddingPayment(false)
    setPaymentForm({ number: '', expiry: '', cvc: '' })
    setPaymentNotice('')
  }
  const addPayment = () => {
    setAddingPayment(true)
    setPaymentNotice('')
  }
  const savePayment = (event: FormEvent) => {
    event.preventDefault()
    const number = paymentForm.number.replace(/\D/g, '')
    const expiry = paymentForm.expiry.trim()
    if (number.length < 13 || number.length > 19) {
      setPaymentNotice('Enter a valid card number')
      return
    }
    if (!/^(0[1-9]|1[0-2])\s*\/\s*\d{2}$/.test(expiry)) {
      setPaymentNotice('Enter a valid expiry as MM / YY')
      return
    }
    if (paymentForm.cvc.replace(/\D/g, '').length < 3) {
      setPaymentNotice('Enter the 3-digit security code')
      return
    }
    const brand = number.startsWith('4') ? 'Visa' : number.startsWith('5') ? 'Mastercard' : 'Card'
    const method: PaymentMethod = {
      id: `pm-${Date.now()}`,
      brand,
      last4: number.slice(-4),
      expiry: `${expiry.slice(0, 2)} / ${expiry.slice(-2)}`,
      default: methods.length === 0,
    }
    setMethods((current) => [...current, method])
    auditApi.add('billing', 'Payment method updated', `Added ${brand} ending in ${method.last4}`)
    resetPaymentForm()
    setPaymentNotice('Payment method added')
    window.setTimeout(() => setPaymentNotice(''), 4000)
  }
  const makeDefault = (id: string) =>
    setMethods((current) => current.map((method) => ({ ...method, default: method.id === id })))
  const removePayment = (id: string) => {
    const target = methods.find((method) => method.id === id)
    const next = methods.filter((method) => method.id !== id)
    setMethods(
      next.length === 0
        ? next
        : next.map((method) => ({
            ...method,
            default: method.default || next[0].id === method.id,
          })),
    )
    if (target)
      auditApi.add(
        'billing',
        'Payment method removed',
        `Removed ${target.brand} ending in ${target.last4}`,
      )
  }

  return (
    <div className="settings-page" role="region" aria-label="Billing">
      <header className="calendar-head">
        <div>
          <p className="eyebrow">Harbor Mail</p>
          <h1>Billing</h1>
        </div>
        <div className="calendar-head__actions">
          <button
            type="button"
            className="secondary-button"
            onClick={() => navigate('/mail/pricing')}
          >
            Compare plans
          </button>
          <button type="button" className="secondary-button" onClick={back}>
            <ArrowLeft size={14} />
            Back to inbox
          </button>
        </div>
      </header>

      <nav className="admin-nav" aria-label="Billing sections">
        <button
          type="button"
          className={tab === 'plan' ? 'admin-nav--active' : ''}
          aria-current={tab === 'plan' ? 'page' : undefined}
          onClick={() => setTab('plan')}
        >
          <CreditCard size={14} />
          Plan
        </button>
        <button
          type="button"
          className={tab === 'invoices' ? 'admin-nav--active' : ''}
          aria-current={tab === 'invoices' ? 'page' : undefined}
          onClick={() => setTab('invoices')}
        >
          <FileText size={14} />
          Invoices
        </button>
      </nav>

      {tab === 'plan' && (
        <>
          <div className="settings-section">
            <h2>Current plan</h2>
            <div className="billing-plan">
              <div>
                <strong>{current.name}</strong>
                <small>
                  <span>{current.price} per seat / month</span> · Renews October 1, 2026
                </small>
              </div>
              <button
                type="button"
                className="secondary-button"
                onClick={() => setChanging((value) => !value)}
              >
                {changing ? 'Cancel' : 'Change plan'}
              </button>
            </div>
            {planNotice && <p className="settings-notice settings-notice--ok">{planNotice}</p>}
            <div className="storage">
              <span>Seats used</span>
              <strong>
                1 <small>/ {current.seats}</small>
              </strong>
              <div className="storage-bar">
                <span style={{ width: `${(1 / current.seats) * 100}%` }} />
              </div>
            </div>
          </div>
          {changing && (
            <div className="settings-section">
              <h2>Choose a plan</h2>
              {plans.map((item) => (
                <button
                  type="button"
                  className={`plan-option ${plan === item.id ? 'plan-option--active' : ''}`}
                  key={item.id}
                  onClick={() => setPlan(item.id)}
                  aria-pressed={plan === item.id}
                >
                  <span>
                    <strong>{item.name}</strong>
                    <small>{item.detail}</small>
                  </span>
                  <span className="plan-option__price">
                    <strong>{item.price}</strong>
                    <small>per seat / month</small>
                  </span>
                  {plan === item.id && <Check size={15} />}
                </button>
              ))}
              <div className="row-actions">
                <button
                  type="button"
                  className="secondary-button"
                  onClick={() => navigate('/mail/pricing')}
                >
                  Compare all plans
                </button>
                <button type="button" className="primary-button" onClick={applyPlan}>
                  Apply plan
                </button>
              </div>
            </div>
          )}
          <div className="settings-section">
            <h2>Payment methods</h2>
            {methods.map((method) => (
              <div className="billing-row" key={method.id}>
                <div>
                  <strong>
                    {method.brand} ending in {method.last4}
                  </strong>
                  <small>Expires {method.expiry}</small>
                </div>
                {method.default && (
                  <span className="billing-paid">
                    <Star size={13} />
                    Default
                  </span>
                )}
                <div className="admin-actions">
                  {!method.default && (
                    <button
                      type="button"
                      className="secondary-button"
                      onClick={() => makeDefault(method.id)}
                    >
                      <Star size={13} />
                      Make default
                    </button>
                  )}
                  <button
                    type="button"
                    className="icon-button"
                    aria-label={`Remove ${method.brand} ending in ${method.last4}`}
                    onClick={() => removePayment(method.id)}
                  >
                    <Trash2 size={15} />
                  </button>
                </div>
              </div>
            ))}
            {methods.length === 0 && (
              <p className="settings-hint">
                No saved payment methods. Add one to keep billing active.
              </p>
            )}
            {paymentNotice && (
              <p className="settings-notice settings-notice--ok">{paymentNotice}</p>
            )}
            {addingPayment ? (
              <form className="payment-form" onSubmit={savePayment}>
                <label>
                  Card number
                  <input
                    inputMode="numeric"
                    autoComplete="cc-number"
                    value={paymentForm.number}
                    onChange={(event) =>
                      setPaymentForm((current) => ({ ...current, number: event.target.value }))
                    }
                    placeholder="1234 5678 9012 3456"
                    aria-label="Card number"
                  />
                </label>
                <div className="payment-form__row">
                  <label>
                    Expiry (MM / YY)
                    <input
                      inputMode="numeric"
                      autoComplete="cc-exp"
                      value={paymentForm.expiry}
                      onChange={(event) =>
                        setPaymentForm((current) => ({ ...current, expiry: event.target.value }))
                      }
                      placeholder="09 / 28"
                      aria-label="Expiry"
                    />
                  </label>
                  <label>
                    Security code
                    <input
                      inputMode="numeric"
                      autoComplete="cc-csc"
                      type="password"
                      value={paymentForm.cvc}
                      onChange={(event) =>
                        setPaymentForm((current) => ({ ...current, cvc: event.target.value }))
                      }
                      placeholder="123"
                      aria-label="Security code"
                    />
                  </label>
                </div>
                <div className="row-actions">
                  <button type="button" className="secondary-button" onClick={resetPaymentForm}>
                    Cancel
                  </button>
                  <button className="primary-button">Save card</button>
                </div>
              </form>
            ) : (
              <div className="row-actions">
                <button type="button" className="secondary-button" onClick={addPayment}>
                  <Plus size={15} />
                  Add payment method
                </button>
              </div>
            )}
          </div>
          <footer>
            <button type="button" className="primary-button" onClick={back}>
              Done
            </button>
          </footer>
        </>
      )}

      {tab === 'invoices' && (
        <div className="settings-section">
          <div className="admin-section-head">
            <h2>Invoices</h2>
            <span className="admin-section-count">{invoices.length} invoices</span>
          </div>
          {invoices.map((invoice) => (
            <div className="billing-row" key={invoice.id}>
              <div>
                <strong>{invoice.id}</strong>
                <small>
                  {invoice.date} · {invoice.amount}
                </small>
              </div>
              <span className="billing-paid">
                <Check size={13} />
                Paid
              </span>
              <div className="admin-actions">
                {downloaded.includes(invoice.id) ? (
                  <small className="settings-notice settings-notice--ok">Sent to your inbox</small>
                ) : (
                  <button
                    type="button"
                    className="secondary-button"
                    onClick={() => downloadInvoice(invoice.id)}
                  >
                    <Download size={13} />
                    Download
                  </button>
                )}
                <button
                  type="button"
                  className="secondary-button"
                  onClick={() => navigate(`/mail/billing/invoices/${invoice.id}`)}
                >
                  <FileText size={13} />
                  View receipt
                </button>
              </div>
            </div>
          ))}
          <footer>
            <button type="button" className="primary-button" onClick={back}>
              Done
            </button>
          </footer>
        </div>
      )}
    </div>
  )
}
