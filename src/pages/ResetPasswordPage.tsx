import { useState, type FormEvent } from 'react'
import { ArrowRight, Eye, EyeOff, KeyRound, ShieldCheck } from 'lucide-react'
import { Link, useSearchParams } from 'react-router-dom'
import { AuthShell } from '../components/AuthShell'
import { authApi } from '../services/auth'
import { auditApi } from '../services/audit'

export function ResetPasswordPage() {
  const [searchParams] = useSearchParams()
  const token = searchParams.get('token') || ''
  const [passwordVisible, setPasswordVisible] = useState(false)
  const [error, setError] = useState('')
  const [reset, setReset] = useState(false)
  const [loading, setLoading] = useState(false)
  const submit = async (event: FormEvent) => {
    event.preventDefault()
    const form = new FormData(event.currentTarget as HTMLFormElement)
    const password = String(form.get('password') || '')
    const confirm = String(form.get('confirm') || '')
    setError('')
    if (password.length < 6) return setError('Password must be at least 6 characters')
    if (password !== confirm) return setError('Passwords do not match')
    setLoading(true)
    try {
      await authApi.resetPassword(token, password)
      auditApi.add('security', 'Password reset', 'Password changed via reset link')
      setReset(true)
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : 'Unable to reset your password')
    } finally {
      setLoading(false)
    }
  }
  return (
    <AuthShell
      eyebrow="Account access"
      title="Choose a new password"
      copy="Pick a fresh password for your account."
      asideIcon={<KeyRound size={20} />}
      foot={
        <Link to="/login" className="text-button">
          <KeyRound size={13} />
          Back to sign in
        </Link>
      }
    >
      {reset ? (
        <div className="auth-status" role="status">
          <span className="auth-status__icon auth-status__icon--success">
            <ShieldCheck size={22} />
          </span>
          <strong>Password updated</strong>
          <p>Your password has been changed. You can now sign in with your new password.</p>
          <Link className="primary-button auth-submit" to="/login">
            Go to sign in
            <ArrowRight size={16} />
          </Link>
        </div>
      ) : token ? (
        <form onSubmit={submit}>
          <label>
            New password
            <span className="password-field">
              <input
                name="password"
                type={passwordVisible ? 'text' : 'password'}
                autoComplete="new-password"
                placeholder="At least 6 characters"
                required
              />
              <button
                type="button"
                onClick={() => setPasswordVisible((value) => !value)}
                aria-label={passwordVisible ? 'Hide password' : 'Show password'}
              >
                {passwordVisible ? <EyeOff size={16} /> : <Eye size={16} />}
              </button>
            </span>
          </label>
          <label>
            Confirm new password
            <input
              name="confirm"
              type={passwordVisible ? 'text' : 'password'}
              autoComplete="new-password"
              placeholder="Repeat your password"
              required
            />
          </label>
          {error && (
            <p className="form-error" role="alert">
              {error}
            </p>
          )}
          <button className="primary-button auth-submit" disabled={loading}>
            {loading ? 'Working...' : 'Update password'}
            {!loading && <ArrowRight size={16} />}
          </button>
        </form>
      ) : (
        <div className="auth-status" role="alert">
          <span className="auth-status__icon">
            <KeyRound size={22} />
          </span>
          <strong>Link invalid or expired</strong>
          <p>
            This password reset link is missing or no longer valid. Request a fresh link to
            continue.
          </p>
          <Link className="primary-button auth-submit" to="/forgot-password">
            Request a new link
          </Link>
        </div>
      )}
    </AuthShell>
  )
}
