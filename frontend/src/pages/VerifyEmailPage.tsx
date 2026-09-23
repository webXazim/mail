import { useState, type FormEvent } from 'react'
import { ArrowRight, BadgeCheck, KeyRound, UserPlus } from 'lucide-react'
import { Link, useLocation, useSearchParams } from 'react-router-dom'
import { AuthShell } from '../components/AuthShell'
import { authApi } from '../services/auth'
import { auditApi } from '../services/audit'

export function VerifyEmailPage() {
  const [searchParams] = useSearchParams()
  const location = useLocation()
  const token = searchParams.get('token') || ''
  const initialEmail = (location.state as { email?: string } | null)?.email || ''
  const [error, setError] = useState('')
  const [sentTo, setSentTo] = useState('')
  const [verified, setVerified] = useState(false)
  const [loading, setLoading] = useState(false)
  const resend = async (event: FormEvent) => {
    event.preventDefault()
    const form = new FormData(event.currentTarget as HTMLFormElement)
    const email = String(form.get('email') || '')
    setError('')
    setSentTo('')
    if (!email.includes('@')) return setError('Enter a valid email address')
    setLoading(true)
    try {
      await authApi.resendVerification(email)
      setSentTo(email)
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : 'Unable to resend the verification email')
    } finally {
      setLoading(false)
    }
  }
  const confirm = async (event: FormEvent) => {
    event.preventDefault()
    setError('')
    setLoading(true)
    try {
      await authApi.verifyEmail(token)
      auditApi.add('security', 'Email verified', 'You confirmed your email address')
      setVerified(true)
    } catch (cause) {
      setError(
        cause instanceof Error ? cause.message : 'This verification link is invalid or has expired',
      )
    } finally {
      setLoading(false)
    }
  }
  return (
    <AuthShell
      eyebrow="Account access"
      title="Verify your email"
      copy="Confirm the login email on your CS Mail account before creating or joining a business."
      asideIcon={<UserPlus size={20} />}
      foot={
        <Link to="/login" className="text-button">
          <KeyRound size={13} />
          Back to sign in
        </Link>
      }
    >
      {verified ? (
        <div className="auth-status" role="status">
          <span className="auth-status__icon auth-status__icon--success">
            <BadgeCheck size={22} />
          </span>
          <strong>Email verified</strong>
          <p>Your login email is confirmed. Sign in to create a business or accept an invitation.</p>
          <Link className="primary-button auth-submit" to="/login">
            Go to sign in
            <ArrowRight size={16} />
          </Link>
        </div>
      ) : token ? (
        <div className="auth-status">
          <span className="auth-status__icon">
            <KeyRound size={22} />
          </span>
          <strong>Verify your account</strong>
          <p>Your verification link is ready. Confirm your email to continue.</p>
          <form onSubmit={confirm}>
            {error && (
              <p className="form-error" role="alert">
                {error}
              </p>
            )}
            <button className="primary-button auth-submit" disabled={loading}>
              {loading ? 'Working...' : 'Confirm email'}
              {!loading && <ArrowRight size={16} />}
            </button>
          </form>
        </div>
      ) : (
        <form onSubmit={resend}>
          <p className="auth-status" role="status">
            Check your inbox and spam folder for the verification link. If it has not arrived, request a new one below.
          </p>
          <label>
            Email address
            <input
              name="email"
              type="email"
              autoComplete="email"
              defaultValue={initialEmail}
              placeholder="you@example.com"
              required
            />
          </label>
          {error && (
            <p className="form-error" role="alert">
              {error}
            </p>
          )}
          {sentTo ? (
            <p className="form-success" role="status">
              If an account exists for {sentTo}, a verification email is on its way.
            </p>
          ) : null}
          <button className="primary-button auth-submit" disabled={loading}>
            {loading ? 'Working...' : 'Resend verification email'}
            {!loading && <ArrowRight size={16} />}
          </button>
        </form>
      )}
    </AuthShell>
  )
}
