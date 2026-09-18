import { useState, type FormEvent } from 'react'
import { ArrowRight, Eye, EyeOff, KeyRound, UserPlus } from 'lucide-react'
import { Link, useNavigate } from 'react-router-dom'
import { AuthShell } from '../components/AuthShell'
import { authApi } from '../services/auth'

export function CreateAccountPage() {
  const navigate = useNavigate()
  const [passwordVisible, setPasswordVisible] = useState(false)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState('')
  const submit = async (event: FormEvent) => {
    event.preventDefault()
    const form = new FormData(event.currentTarget as HTMLFormElement)
    const name = String(form.get('name') || '')
    const email = String(form.get('email') || '')
    const password = String(form.get('password') || '')
    const confirm = String(form.get('confirm') || '')
    setError('')
    if (!name.trim()) return setError('Enter your name')
    if (!email.includes('@')) return setError('Enter a valid email address')
    if (password.length < 12) return setError('Password must be at least 12 characters')
    if (password !== confirm) return setError('Passwords do not match')
    setLoading(true)
    try {
      const result = await authApi.register(name.trim(), email, password)
      if ('access' in result && result.access) {
        localStorage.setItem('harbor-mail:display-name', name.trim())
        navigate('/mail/inbox')
      } else {
        navigate('/verify-email')
      }
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : 'Unable to create account')
    } finally {
      setLoading(false)
    }
  }
  return (
    <AuthShell
      eyebrow="Get started"
      title="Create your account"
      copy="Set up Harbor Mail for you and your team."
      asideIcon={<UserPlus size={20} />}
      foot={
        <Link to="/login" className="text-button">
          <KeyRound size={13} />
          Back to sign in
        </Link>
      }
    >
      <form onSubmit={submit}>
        <label>
          Full name
          <input name="name" type="text" autoComplete="name" placeholder="Alex Morgan" required />
        </label>
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
        <label>
          Password
          <span className="password-field">
            <input
              name="password"
              type={passwordVisible ? 'text' : 'password'}
              autoComplete="new-password"
              placeholder="At least 12 characters"
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
          Confirm password
          <input
            name="confirm"
            type="password"
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
          <ArrowRight size={16} />
          {loading ? 'Creating...' : 'Create account'}
        </button>
      </form>
    </AuthShell>
  )
}
