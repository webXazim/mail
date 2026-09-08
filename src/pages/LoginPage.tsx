import { useState, type FormEvent } from 'react'
import { ArrowRight, Eye, EyeOff, KeyRound, Moon, UserPlus } from 'lucide-react'
import { useNavigate } from 'react-router-dom'
import { authApi } from '../services/auth'

type Mode = 'login' | 'forgot' | 'create'

export function LoginPage() {
  const navigate = useNavigate()
  const [mode, setMode] = useState<Mode>('login')
  const [passwordVisible, setPasswordVisible] = useState(false)
  const [remember, setRemember] = useState(true)
  const [error, setError] = useState('')
  const [notice, setNotice] = useState('')
  const [loading, setLoading] = useState(false)
  const switchMode = (next: Mode) => { setMode(next); setError(''); setNotice('') }
  const setCredentials = () => { localStorage.setItem('harbor-mail:session', 'demo'); navigate('/mail/inbox') }
  const submit = async (event: FormEvent) => {
    event.preventDefault()
    const form = new FormData(event.currentTarget as HTMLFormElement)
    setError(''); setNotice('')
    if (mode === 'forgot') {
      setNotice(`If an account exists for ${String(form.get('email'))}, a reset link is on its way.`)
      return
    }
    if (mode === 'create') {
      const name = String(form.get('name') || '')
      const email = String(form.get('email') || '')
      const password = String(form.get('password') || '')
      const confirm = String(form.get('confirm') || '')
      if (!name.trim()) return setError('Enter your name')
      if (!email.includes('@')) return setError('Enter a valid email address')
      if (password.length < 6) return setError('Password must be at least 6 characters')
      if (password !== confirm) return setError('Passwords do not match')
      localStorage.setItem('harbor-mail:display-name', name.trim())
      setCredentials()
      return
    }
    setLoading(true)
    try {
      await authApi.login(String(form.get('email')), String(form.get('password')))
      if (remember) localStorage.setItem('harbor-mail:session', 'demo')
      navigate('/mail/inbox')
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : 'Unable to sign in')
    } finally {
      setLoading(false)
    }
  }
  return (
    <main className="auth-page">
      <section className="auth-card">
        <div className="auth-brand"><span className="brand-mark">H</span><strong>harbor<span>mail</span></strong></div>
        <p className="eyebrow">{mode === 'login' ? 'Welcome back' : mode === 'forgot' ? 'Account access' : 'Get started'}</p>
        <h1>{mode === 'login' ? 'Sign in to your mailbox' : mode === 'forgot' ? 'Reset your password' : 'Create your account'}</h1>
        <p className="auth-copy">{mode === 'login' ? 'Access your conversations, files, and focused workspaces.' : mode === 'forgot' ? "We'll email you a secure link to change your password." : 'Set up Harbor Mail for you and your team.'}</p>
        <form onSubmit={submit}>
          {mode === 'create' && <label>Full name<input name="name" type="text" autoComplete="name" placeholder="Alex Morgan" required /></label>}
          <label>Email address<input name="email" type="email" autoComplete="email" placeholder="you@company.com" required /></label>
          {mode !== 'forgot' && (
            <label>Password
              <span className="password-field">
                <input name="password" type={passwordVisible ? 'text' : 'password'} autoComplete={mode === 'create' ? 'new-password' : 'current-password'} placeholder="Your password" required />
                <button type="button" onClick={() => setPasswordVisible(value => !value)} aria-label={passwordVisible ? 'Hide password' : 'Show password'}>{passwordVisible ? <EyeOff size={16} /> : <Eye size={16} />}</button>
              </span>
            </label>
          )}
          {mode === 'create' && <label>Confirm password<input name="confirm" type="password" autoComplete="new-password" placeholder="Repeat your password" required /></label>}
          {error && <p className="form-error" role="alert">{error}</p>}
          {notice && <p className="form-success" role="status">{notice}</p>}
          {mode === 'login' && (
            <label className="remember">
              <input type="checkbox" checked={remember} onChange={event => setRemember(event.target.checked)} /><span>Remember this device</span>
            </label>
          )}
          {mode === 'login' && <button type="button" className="text-button auth-forgot" onClick={() => switchMode('forgot')}>Forgot password?</button>}
          <button className="primary-button auth-submit" disabled={loading}>{loading ? 'Working...' : mode === 'login' ? 'Sign in' : mode === 'forgot' ? 'Send reset link' : 'Create account'}{!loading && <ArrowRight size={16} />}</button>
        </form>
        {mode === 'login' && <p className="auth-foot">New to Harbor Mail? <button type="button" className="text-button" onClick={() => switchMode('create')}>Create an account</button></p>}
        {mode !== 'login' && <p className="auth-foot"><button type="button" className="text-button" onClick={() => switchMode('login')}><KeyRound size={13} />Back to sign in</button></p>}
      </section>
      <aside className="auth-aside">{mode === 'login' ? <Moon size={20} /> : <UserPlus size={20} />}<strong>Calm, focused email for teams.</strong><span>Keep important conversations moving without losing the details.</span></aside>
    </main>
  )
}