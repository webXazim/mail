import { useState, type FormEvent } from 'react'
import { ArrowRight, Eye, EyeOff, Moon } from 'lucide-react'
import { Link, useNavigate } from 'react-router-dom'
import { AuthShell } from '../components/AuthShell'
import { authApi } from '../services/auth'
import { auditApi } from '../services/audit'

export function LoginPage() {
  const navigate = useNavigate()
  const [passwordVisible, setPasswordVisible] = useState(false)
  const [remember, setRemember] = useState(true)
  const [error, setError] = useState('')
  const [loading, setLoading] = useState(false)
  const submit = async (event: FormEvent) => {
    event.preventDefault()
    const form = new FormData(event.currentTarget as HTMLFormElement)
    setError('')
    setLoading(true)
    try {
      await authApi.login(String(form.get('email')), String(form.get('password')))
      auditApi.add('sign-in', 'Signed in', 'Chrome on desktop')
      if (remember) localStorage.setItem('harbor-mail:session', 'demo')
      navigate('/mail/inbox')
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : 'Unable to sign in')
    } finally {
      setLoading(false)
    }
  }
  return (
    <AuthShell
      eyebrow="Welcome back"
      title="Sign in to your mailbox"
      copy="Access your conversations, files, and focused workspaces."
      asideIcon={<Moon size={20} />}
      foot={
        <>
          New to Harbor Mail?{' '}
          <Link to="/create-account" className="text-button">
            Create an account
          </Link>
        </>
      }
    >
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
        <label>
          Password
          <span className="password-field">
            <input
              name="password"
              type={passwordVisible ? 'text' : 'password'}
              autoComplete="current-password"
              placeholder="Your password"
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
        {error && (
          <p className="form-error" role="alert">
            {error}
          </p>
        )}
        <label className="remember">
          <input
            type="checkbox"
            checked={remember}
            onChange={(event) => setRemember(event.target.checked)}
          />
          <span>Remember this device</span>
          <Link to="/forgot-password" className="text-button">
            Forgot password?
          </Link>
        </label>
        <button className="primary-button auth-submit" disabled={loading}>
          {loading ? 'Working...' : 'Sign in'}
          {!loading && <ArrowRight size={16} />}
        </button>
      </form>
    </AuthShell>
  )
}
