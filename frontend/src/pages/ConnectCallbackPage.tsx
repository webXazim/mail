import { useEffect, useRef, useState } from 'react'
import { Link, useNavigate } from 'react-router-dom'
import { AuthShell } from '../components/AuthShell'
import { consumeConnectCallback } from '../lib/connect-federation'
import { authApi } from '../services/auth'

export function ConnectCallbackPage() {
  const navigate = useNavigate()
  const [error, setError] = useState('')
  const started = useRef(false)
  useEffect(() => {
    if (started.current) return
    started.current = true
    void Promise.resolve().then(async () => {
      const callback = consumeConnectCallback(location.search)
      await authApi.completeConnect(callback.code, callback.verifier)
      return callback.next
    })
      .then((next) => { if (next) location.replace(next); else navigate('/mail/business', { replace: true }) })
      .catch((cause) => setError(cause instanceof Error ? cause.message : 'Unable to complete sign-in.'))
  }, [navigate])
  return <AuthShell eyebrow="CS Connect" title="Completing sign-in" copy="Opening your CS Mail account.">
    {error ? <><p className="form-error" role="alert">{error}</p><Link className="text-button" to="/login">Back to sign in</Link></> : <p role="status">Checking your account…</p>}
  </AuthShell>
}
