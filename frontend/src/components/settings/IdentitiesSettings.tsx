import { useState } from 'react'
import { Check, Plus, Trash2 } from 'lucide-react'
import { identitiesApi, type Identity } from '../../services/identities'

const matchingEmail = (value: string) => /^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(value.trim())

export function IdentitiesSettings() {
  const [identities, setIdentities] = useState<Identity[]>(() => identitiesApi.list())
  const [name, setName] = useState('')
  const [email, setEmail] = useState('')
  const [notice, setNotice] = useState('')

  const add = () => {
    if (!matchingEmail(email)) {
      setNotice('Enter a valid email address for the new sender.')
      return
    }
    const next = identitiesApi.add({ email, displayName: name })
    if (next.length === identities.length) {
      setNotice('That address is already in use.')
      return
    }
    setIdentities(next)
    setName('')
    setEmail('')
    setNotice('Identity added')
  }

  return (
    <div>
      <div className="settings-section">
        <h3>Send mail as</h3>
        <p className="settings-hint">
          Choose which name and address new messages go out from. The composer "From" menu uses
          these identities.
        </p>
        {identities.map((identity) => (
          <div className="spam-add identity-row" key={identity.id}>
            <input
              value={identity.displayName}
              aria-label={`Name for ${identity.email}`}
              onChange={(event) =>
                setIdentities(identitiesApi.rename(identity.id, event.target.value))
              }
            />
            <code className="identity-email">{identity.email}</code>
            {identity.primary ? (
              <span className="billing-paid">
                <Check size={13} />
                Primary
              </span>
            ) : (
              <button
                type="button"
                className="text-button"
                onClick={() => setIdentities(identitiesApi.setPrimary(identity.id))}
              >
                Make primary
              </button>
            )}
            <button
              type="button"
              className="icon-button"
              aria-label={`Remove ${identity.email}`}
              onClick={() => setIdentities(identitiesApi.remove(identity.id))}
              disabled={identity.primary || identities.length <= 1}
            >
              <Trash2 size={15} />
            </button>
          </div>
        ))}
      </div>

      <div className="settings-section">
        <h3>Add an identity</h3>
        <div className="spam-add">
          <input
            value={name}
            aria-label="Identity name"
            placeholder="Name shown to recipients"
            onChange={(event) => setName(event.target.value)}
          />
          <input
            type="email"
            value={email}
            aria-label="Identity email"
            placeholder="you@example.com"
            onChange={(event) => setEmail(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === 'Enter') {
                event.preventDefault()
                add()
              }
            }}
          />
          <button type="button" className="primary-button" onClick={add}>
            <Plus size={15} />
            Add
          </button>
        </div>
      </div>

      {notice && <p className="settings-notice settings-notice--ok">{notice}</p>}
    </div>
  )
}
