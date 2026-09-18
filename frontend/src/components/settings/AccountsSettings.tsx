import { useState, type FormEvent } from 'react'
import { Plus, Trash2 } from 'lucide-react'
import { useMail } from '../../state/mail/MailContext'
import { primaryAccountId } from '../../services/accounts'

export function AccountsSettings() {
  const { accounts, addAccount, removeAccount } = useMail()
  const [name, setName] = useState('')
  const [email, setEmail] = useState('')
  const [password, setPassword] = useState('')
  const [notice, setNotice] = useState('')

  const submit = (event: FormEvent) => {
    event.preventDefault()
    try {
      addAccount({ name, email, password })
      setName('')
      setEmail('')
      setPassword('')
      setNotice('')
    } catch (error) {
      setNotice(error instanceof Error ? error.message : 'Unable to add that account')
    }
  }

  return (
    <div>
      <div className="settings-section">
        <h3>Linked accounts</h3>
        <p className="settings-hint">
          A unified inbox merges mail from every linked account. Switch between accounts in the
          sidebar.
        </p>
        {accounts.map((account) => (
          <div className="billing-row" key={account.id}>
            <div className="account-identity">
              <i className={`avatar avatar--${account.color}`}>{account.initials}</i>
              <span>
                <strong>{account.name}</strong>
                <small>{account.email}</small>
              </span>
            </div>
            <div className="account-identity__actions">
              {account.id === primaryAccountId ? (
                <span className="badge badge--open">Primary</span>
              ) : (
                <button
                  type="button"
                  className="secondary-button"
                  onClick={() => removeAccount(account.id)}
                >
                  <Trash2 size={14} />
                  Remove
                </button>
              )}
            </div>
          </div>
        ))}
      </div>
      <div className="settings-section">
        <h3>Add an account</h3>
        <form onSubmit={submit}>
          <label>
            Display name
            <input
              value={name}
              onChange={(event) => setName(event.target.value)}
              placeholder="e.g. Amira Khalil"
            />
          </label>
          <label>
            Email address
            <input
              type="email"
              value={email}
              onChange={(event) => setEmail(event.target.value)}
              placeholder="you@anotherdomain.co"
            />
          </label>
          <label>
            Password
            <input
              type="password"
              value={password}
              onChange={(event) => setPassword(event.target.value)}
              placeholder="At least 6 characters"
              autoComplete="new-password"
            />
          </label>
          {notice && <p className="settings-hint">{notice}</p>}
          <button className="primary-button">
            <Plus size={15} />
            Link account
          </button>
        </form>
      </div>
    </div>
  )
}
