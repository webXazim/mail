import { useState, type FormEvent } from 'react'
import { ArrowRight, Eye, EyeOff, KeyRound, UserPlus } from 'lucide-react'
import { Link, useNavigate } from 'react-router-dom'
import { AuthShell } from '../components/AuthShell'

export function CreateAccountPage() {
  const navigate = useNavigate()
  const [passwordVisible, setPasswordVisible] = useState(false)
  const [error, setError] = useState('')
  const submit = (event: FormEvent) => {
    event.preventDefault()
    const form = new FormData(event.currentTarget as HTMLFormElement)
    const name = String(form.get('name') || '')
    const email = String(form.get('email') || '')
    const password = String(form.get('password') || '')
    const confirm = String(form.get('confirm') || '')
    setError('')
    if (!name.trim()) return setError('Enter your name')
    if (!email.includes('@')) return setError('Enter a valid email address')
    if (password.length < 6) return setError('Password must be at least 6 characters')
    if (password !== confirm) return setError('Passwords do not match')
    localStorage.setItem('harbor-mail:display-name', name.trim())
    localStorage.setItem('harbor-mail:session', 'demo')
    navigate('/mail/inbox')
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
        <button className="primary-button auth-submit">
          <ArrowRight size={16} />
          Create account
        </button>
      </form>
    </AuthShell>
  )
}
