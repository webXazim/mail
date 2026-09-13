import { useState, type FormEvent } from 'react'
import { ArrowRight, KeyRound, MailCheck } from 'lucide-react'
import { Link } from 'react-router-dom'
import { AuthShell } from '../components/AuthShell'
import { authApi } from '../services/auth'

export function ForgotPasswordPage() {
  const [error, setError] = useState('')
  const [sentTo, setSentTo] = useState('')
  const [loading, setLoading] = useState(false)
  const submit = async (event: FormEvent) => {
    event.preventDefault()
    const form = new FormData(event.currentTarget as HTMLFormElement)
    const email = String(form.get('email') || '')
    setError('')
    setSentTo('')
    if (!email.includes('@')) return setError('Enter a valid email address')
    setLoading(true)
    try {
      await authApi.requestPasswordReset(email)
      setSentTo(email)
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : 'Unable to send a reset link')
    } finally {
      setLoading(false)
    }
  }
  return (
    <AuthShell
      eyebrow="Account access"
      title="Reset your password"
      copy="We'll email you a secure link to change your password."
      asideIcon={<KeyRound size={20} />}
      foot={
        <Link to="/login" className="text-button">
          <KeyRound size={13} />
          Back to sign in
        </Link>
      }
    >
      {sentTo ? (
        <div className="auth-status" role="status">
          <span className="auth-status__icon">
            <MailCheck size={22} />
          </span>
          <strong>Check your inbox</strong>
          <p>
            If an account exists for <strong>{sentTo}</strong>, a reset link is on its way. The link
            expires in 30 minutes.
          </p>
          <button className="primary-button auth-submit" onClick={() => setSentTo('')}>
            Send another link
          </button>
        </div>
      ) : (
        <form onSubmit={submit}>
          <label>
            Email address
            <input
              name="email"
              type="email"
              autoComplete="email"
              placeholder="you@company.com"
              required
            />
          </label>
          {error && (
            <p className="form-error" role="alert">
              {error}
            </p>
          )}
          <button className="primary-button auth-submit" disabled={loading}>
            {loading ? 'Working...' : 'Send reset link'}
            {!loading && <ArrowRight size={16} />}
          </button>
        </form>
      )}
    </AuthShell>
  )
}
