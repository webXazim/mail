import { useState, type FormEvent } from 'react'
import { ArrowLeft, ArrowRight, Eye, EyeOff, Moon, ShieldCheck } from 'lucide-react'
import { Link, useLocation, useNavigate } from 'react-router-dom'
import { AuthShell } from '../components/AuthShell'
import { authApi } from '../services/auth'
import { auditApi } from '../services/audit'
import { profileApi } from '../services/profile'

export function LoginPage() {
  const navigate = useNavigate()
  const location = useLocation()
  const requested = (location.state as { from?: string } | null)?.from || sessionStorage.getItem('cs-mail:return-to') || undefined
  const continueAfterLogin = async () => {
    if (requested) {
      sessionStorage.removeItem('cs-mail:return-to')
      navigate(requested, { replace: true })
      return
    }
    const profile = await profileApi.refresh().catch(() => null)
    navigate(profile?.has_mailbox === false ? '/mail/business' : '/mail/inbox', { replace: true })
  }
  const [passwordVisible, setPasswordVisible] = useState(false)
  const [remember, setRemember] = useState(true)
  const [error, setError] = useState('')
  const [loading, setLoading] = useState(false)
  const [challengeToken, setChallengeToken] = useState('')
  const [factorCode, setFactorCode] = useState('')

  const submit = async (event: FormEvent) => {
    event.preventDefault()
    const form = new FormData(event.currentTarget as HTMLFormElement)
    setError('')
    setLoading(true)
    try {
      const result = await authApi.login(String(form.get('email')), String(form.get('password')))
      if ('two_factor_required' in result) {
        setChallengeToken(result.challenge_token)
        setFactorCode('')
        return
      }
      auditApi.add('sign-in', 'Signed in', 'Browser session')
      await continueAfterLogin()
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : 'Unable to sign in')
    } finally {
      setLoading(false)
    }
  }

  const verifyFactor = async (event: FormEvent) => {
    event.preventDefault()
    setError('')
    setLoading(true)
    try {
      const result = await authApi.verifyTwoFactor(challengeToken, factorCode)
      auditApi.add(
        'sign-in',
        'Signed in with two-factor authentication',
        result.recovery_code_used ? 'Recovery code used' : 'Authenticator code used',
      )
      await continueAfterLogin()
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : 'Unable to verify authentication code')
    } finally {
      setLoading(false)
    }
  }

  return (
    <AuthShell
      eyebrow={challengeToken ? 'Two-factor authentication' : 'Welcome back'}
      title={challengeToken ? 'Verify your sign-in' : 'Sign in to CS Mail'}
      copy={
        challengeToken
          ? 'Enter the six-digit code from your authenticator app, or use one of your recovery codes.'
          : 'Access your businesses, mailboxes, conversations, and account settings.'
      }
      asideIcon={challengeToken ? <ShieldCheck size={20} /> : <Moon size={20} />}
      foot={
        challengeToken ? (
          <button
            type="button"
            className="text-button"
            onClick={() => {
              setChallengeToken('')
              setFactorCode('')
              setError('')
            }}
          >
            <ArrowLeft size={14} /> Back to password sign-in
          </button>
        ) : (
          <>
            New to CS Mail?{' '}
            <Link to="/create-account" state={requested ? { from: requested } : undefined} className="text-button">
              Create an account
            </Link>
          </>
        )
      }
    >
      {challengeToken ? (
        <form onSubmit={verifyFactor}>
          <label>
            Authentication code
            <input
              name="factor"
              type="text"
              inputMode="text"
              autoComplete="one-time-code"
              placeholder="123456 or recovery code"
              value={factorCode}
              onChange={(event) => setFactorCode(event.target.value)}
              required
              autoFocus
            />
          </label>
          {error && (
            <p className="form-error" role="alert">
              {error}
            </p>
          )}
          <p className="settings-hint">
            Recovery codes are single-use. After using one, generate a replacement set from Security settings.
          </p>
          <button className="primary-button auth-submit" disabled={loading || !factorCode.trim()}>
            {loading ? 'Verifying...' : 'Verify and sign in'}
            {!loading && <ArrowRight size={16} />}
          </button>
        </form>
      ) : (
        <form onSubmit={submit}>
          <label>
            Email address
            <input
              name="email"
              type="email"
              autoComplete="email"
              placeholder="you@example.com"
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
      )}
    </AuthShell>
  )
}
