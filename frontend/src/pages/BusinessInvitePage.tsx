import { useEffect, useState } from 'react'
import { Building2, CheckCircle2 } from 'lucide-react'
import { Link, useNavigate, useSearchParams } from 'react-router-dom'
import { organizationsApi } from '../services/organizations'
import { profileApi } from '../services/profile'

export function BusinessInvitePage() {
  const [params] = useSearchParams()
  const navigate = useNavigate()
  const token = params.get('token') || ''
  const mailboxToken = params.get('mailbox_token') || ''
  const [status, setStatus] = useState<'working' | 'done' | 'error'>('working')
  const [message, setMessage] = useState('Accepting your business invitation…')

  useEffect(() => {
    if (!token && !mailboxToken) {
      setStatus('error')
      setMessage('This invitation link is missing its token.')
      return
    }
    const accept = mailboxToken ? organizationsApi.acceptMailboxInvitation(mailboxToken) : organizationsApi.acceptInvitation(token)
    accept
      .then(async (result) => {
        await organizationsApi.activate(result.organization_id)
        await profileApi.refresh()
        setStatus('done')
        setMessage('Invitation accepted. Your business workspace is ready.')
        window.setTimeout(() => navigate('/mail/business', { replace: true }), 900)
      })
      .catch((cause) => {
        setStatus('error')
        setMessage(cause instanceof Error ? cause.message : 'Unable to accept this invitation')
      })
  }, [token, mailboxToken, navigate])

  return (
    <main className="invite-page">
      <div className="invite-card">
        <span className="invite-icon">{status === 'done' ? <CheckCircle2 size={24} /> : <Building2 size={24} />}</span>
        <h1>{status === 'done' ? 'Business joined' : 'Business invitation'}</h1>
        <p>{message}</p>
        {status === 'error' && <Link className="primary-button" to="/mail/business">Open businesses</Link>}
      </div>
    </main>
  )
}
