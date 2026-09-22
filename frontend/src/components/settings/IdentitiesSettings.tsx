import { useEffect, useState } from 'react'
import { Check, Plus, RefreshCw, Trash2 } from 'lucide-react'
import { identitiesApi, type Identity } from '../../services/identities'
import { isRemoteMail } from '../../services/remote-mail'
import type { RealtimeEvent } from '../../services/ws'

const matchingEmail = (value: string) => /^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(value.trim())

export function IdentitiesSettings() {
  const remote = isRemoteMail()
  const [identities, setIdentities] = useState<Identity[]>(() => identitiesApi.list())
  const [name, setName] = useState('')
  const [email, setEmail] = useState('')
  const [notice, setNotice] = useState('')
  const [verificationCodes, setVerificationCodes] = useState<Record<string, string>>({})
  const [busyId, setBusyId] = useState('')
  const [adding, setAdding] = useState(false)

  useEffect(() => {
    if (!remote) return
    void identitiesApi
      .refresh()
      .then(setIdentities)
      .catch(() => setNotice('Could not load sender identities.'))
  }, [remote])

  useEffect(() => {
    if (!remote) return
    const onRealtime = (incoming: Event) => {
      const detail = (incoming as CustomEvent<RealtimeEvent>).detail
      if (detail?.kind !== 'resource-changed' || detail.payload.resource !== 'identities') return
      void identitiesApi.refresh().then(setIdentities).catch(() => {})
    }
    window.addEventListener('cs-mail-realtime', onRealtime)
    return () => window.removeEventListener('cs-mail-realtime', onRealtime)
  }, [remote])

  const updateName = async (identity: Identity, displayName: string) => {
    if (!remote) {
      setIdentities(identitiesApi.rename(identity.id, displayName))
      return
    }
    setIdentities((current) =>
      current.map((item) => (item.id === identity.id ? { ...item, displayName } : item)),
    )
    try {
      setIdentities(await identitiesApi.update(identity.id, { displayName }))
      setNotice('Sender name updated')
    } catch {
      setNotice('Could not update that sender name.')
      setIdentities(await identitiesApi.refresh().catch(() => identitiesApi.list()))
    }
  }

  const add = async () => {
    if (!matchingEmail(email)) {
      setNotice('Enter a valid email address for the new sender.')
      return
    }
    if (!remote) {
      const next = identitiesApi.add({ email, displayName: name })
      if (next.length === identities.length) {
        setNotice('That address is already in use.')
        return
      }
      setIdentities(next)
      setName('')
      setEmail('')
      setNotice('Identity added')
      return
    }

    setAdding(true)
    try {
      const result = await identitiesApi.create({ email, displayName: name })
      setIdentities(result.identities)
      const created = result.identities.find(
        (identity) => identity.email.toLowerCase() === email.trim().toLowerCase(),
      )
      if (created && result.verificationCode) {
        setVerificationCodes((current) => ({ ...current, [created.id]: result.verificationCode! }))
      }
      setName('')
      setEmail('')
      setNotice(
        created?.status === 'pending'
          ? 'Verification code sent to that address.'
          : 'Verified identity added.',
      )
    } catch (error) {
      setNotice(error instanceof Error ? error.message : 'Could not add that sender identity.')
    } finally {
      setAdding(false)
    }
  }

  const verify = async (identity: Identity) => {
    const code = (verificationCodes[identity.id] ?? '').trim()
    if (!code) {
      setNotice('Enter the verification code sent to that address.')
      return
    }
    setBusyId(identity.id)
    try {
      setIdentities(await identitiesApi.verify(identity.id, code))
      setVerificationCodes((current) => {
        const next = { ...current }
        delete next[identity.id]
        return next
      })
      setNotice(`${identity.email} is verified. External addresses are available for Reply-To; From stays on your hosted business addresses.`)
    } catch (error) {
      setNotice(error instanceof Error ? error.message : 'Could not verify that identity.')
    } finally {
      setBusyId('')
    }
  }

  const resend = async (identity: Identity) => {
    setBusyId(identity.id)
    try {
      const code = await identitiesApi.resend(identity.id)
      if (code) {
        setVerificationCodes((current) => ({ ...current, [identity.id]: code }))
      }
      setNotice('A new verification code was sent.')
    } catch (error) {
      setNotice(error instanceof Error ? error.message : 'Could not resend the verification code.')
    } finally {
      setBusyId('')
    }
  }

  const makeDefault = async (identity: Identity) => {
    setBusyId(identity.id)
    try {
      setIdentities(
        remote
          ? await identitiesApi.makeDefault(identity.id)
          : identitiesApi.setPrimary(identity.id),
      )
      setNotice('Default sender updated.')
    } catch (error) {
      setNotice(error instanceof Error ? error.message : 'Could not change the default sender.')
    } finally {
      setBusyId('')
    }
  }

  const remove = async (identity: Identity) => {
    setBusyId(identity.id)
    try {
      setIdentities(
        remote ? await identitiesApi.removeRemote(identity.id) : identitiesApi.remove(identity.id),
      )
      setNotice('Sender identity removed.')
    } catch (error) {
      setNotice(error instanceof Error ? error.message : 'Could not remove that sender identity.')
    } finally {
      setBusyId('')
    }
  }

  return (
    <div>
      <div className="settings-section">
        <h3>Send mail as</h3>
        <p className="settings-hint">
          From addresses must be hosted CS Mail business mailboxes or aliases on a DNS-ready
          domain. External addresses may be verified for Reply-To, but cannot be used as From.
        </p>
        {identities.map((identity) => {
          const pending = remote && identity.status === 'pending'
          const verified = !remote || identity.status === 'verified' || !identity.status
          const hostedFrom = !remote || identity.source === 'primary' || identity.source === 'alias'
          const removable = remote ? identity.source !== 'primary' : !identity.primary && identities.length > 1
          return (
            <div className="spam-add identity-row" key={identity.id}>
              <input
                value={identity.displayName}
                aria-label={`Name for ${identity.email}`}
                onChange={(event) => {
                  const value = event.target.value
                  if (remote) {
                    setIdentities((current) =>
                      current.map((item) =>
                        item.id === identity.id ? { ...item, displayName: value } : item,
                      ),
                    )
                  } else {
                    setIdentities(identitiesApi.rename(identity.id, value))
                  }
                }}
                onBlur={(event) => void updateName(identity, event.target.value)}
              />
              <code className="identity-email">{identity.email}</code>
              {remote && (
                <input
                  type="email"
                  aria-label={`Reply-To for ${identity.email}`}
                  defaultValue={identity.replyTo || ''}
                  placeholder="Reply-To (optional)"
                  onBlur={(event) => {
                    const value = event.target.value.trim()
                    void identitiesApi
                      .update(identity.id, { replyTo: value || null })
                      .then(setIdentities)
                      .then(() => setNotice('Reply-To updated'))
                      .catch(() => setNotice('Could not update Reply-To.'))
                  }}
                />
              )}
              {identity.primary && hostedFrom ? (
                <span className="billing-paid">
                  <Check size={13} />
                  Default
                </span>
              ) : verified && hostedFrom ? (
                <button
                  type="button"
                  className="text-button"
                  disabled={busyId === identity.id}
                  onClick={() => void makeDefault(identity)}
                >
                  Make default
                </button>
              ) : verified ? (
                <span className="settings-hint">Reply-To only</span>
              ) : (
                <span className="settings-hint">Pending verification</span>
              )}
              {removable && (
                <button
                  type="button"
                  className="icon-button"
                  aria-label={`Remove ${identity.email}`}
                  disabled={busyId === identity.id}
                  onClick={() => void remove(identity)}
                >
                  <Trash2 size={15} />
                </button>
              )}
              {pending && (
                <>
                  <input
                    value={verificationCodes[identity.id] ?? ''}
                    aria-label={`Verification code for ${identity.email}`}
                    placeholder="Verification code"
                    autoComplete="one-time-code"
                    onChange={(event) =>
                      setVerificationCodes((current) => ({
                        ...current,
                        [identity.id]: event.target.value,
                      }))
                    }
                    onKeyDown={(event) => {
                      if (event.key === 'Enter') {
                        event.preventDefault()
                        void verify(identity)
                      }
                    }}
                  />
                  <button
                    type="button"
                    className="primary-button"
                    disabled={busyId === identity.id}
                    onClick={() => void verify(identity)}
                  >
                    Verify
                  </button>
                  <button
                    type="button"
                    className="text-button"
                    disabled={busyId === identity.id}
                    onClick={() => void resend(identity)}
                  >
                    <RefreshCw size={14} />
                    Resend
                  </button>
                </>
              )}
            </div>
          )
        })}
      </div>

      <div className="settings-section">
        <h3>Add an identity</h3>
        {remote && (
          <p className="settings-hint">
            External addresses receive a one-time verification code. Managed aliases assigned by
            an administrator are available without an email challenge.
          </p>
        )}
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
                void add()
              }
            }}
          />
          <button type="button" className="primary-button" disabled={adding} onClick={() => void add()}>
            <Plus size={15} />
            {adding ? 'Adding…' : 'Add'}
          </button>
        </div>
      </div>

      {notice && <p className="settings-notice settings-notice--ok">{notice}</p>}
    </div>
  )
}
