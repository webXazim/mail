import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type ClipboardEvent,
  type DragEvent,
  type FormEvent,
} from 'react'
import { Clock3, Paperclip, Send, X } from 'lucide-react'
import { useFocusTrap } from '../hooks/useFocusTrap'
import { deleteStagedAttachment, uploadAttachment } from '../services/attachments'
import { contactsService } from '../services/contacts'
import { draftKey, draftsApi } from '../services/drafts'
import { identitiesApi, type Identity } from '../services/identities'
import { parseAddresses } from '../lib/mail'
import { isRemoteMail, parseRecipients, remoteDraftApi } from '../services/remote-mail'
import { settingsApi } from '../services/settings'
import type { Draft, DraftAttachment } from '../types'

type ComposerProps = {
  close: () => void
  onSent?: (draft: Draft) => void | Promise<boolean>
  initialDraft?: Partial<Draft>
}
const freshKey = () => crypto.randomUUID()
const emptyDraft: Draft = {
  to: '',
  cc: '',
  bcc: '',
  subject: '',
  body: '',
  attachments: [],
  scheduledAt: '',
}
const normalizeAttachments = (value: unknown): DraftAttachment[] =>
  Array.isArray(value)
    ? value.filter((item): item is DraftAttachment =>
        Boolean(
          item &&
          typeof item === 'object' &&
          'id' in item &&
          typeof item.id === 'string' &&
          'filename' in item &&
          typeof item.filename === 'string',
        ),
      )
    : []

const addressIsInvalid = (part: string) => {
  const addr = part.includes('<') ? (part.match(/<([^>]+)>/)?.[1] ?? part) : part
  return !/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(addr)
}

const escapeRegExp = (value: string) => value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')

export function Composer({ close, onSent, initialDraft }: ComposerProps) {
  const signature = settingsApi.load().signature
  const signatureBlock = signature ? `\n--\n${signature}` : ''
  const remote = isRemoteMail()
  const [identities, setIdentities] = useState<Identity[]>(() => identitiesApi.list())
  const [draft, setDraft] = useState<Draft>(() => {
    if (initialDraft) {
      return {
        ...emptyDraft,
        ...initialDraft,
        attachments: normalizeAttachments(initialDraft.attachments),
        clientKey: initialDraft.clientKey || freshKey(),
        sendKey: initialDraft.sendKey || freshKey(),
      }
    }
    let saved: Partial<Draft> = {}
    if (!remote) {
      try {
        saved = JSON.parse(localStorage.getItem(draftKey) || '{}')
      } catch {
        /* ignore a malformed demo autosave */
      }
    }
    if (signature && !saved.body) saved.body = signatureBlock
    return {
      ...emptyDraft,
      ...saved,
      attachments: normalizeAttachments(saved.attachments),
      clientKey: saved.clientKey || freshKey(),
      sendKey: saved.sendKey || freshKey(),
    }
  })
  const [includeSignature, setIncludeSignature] = useState(Boolean(signature))
  const [showCopies, setShowCopies] = useState(Boolean(draft.cc || draft.bcc))
  const [status, setStatus] = useState(remote ? 'Ready' : 'Saved to Drafts')
  const [uploading, setUploading] = useState(false)
  const [sending, setSending] = useState(false)
  const sendingRef = useRef(false)
  const [dragging, setDragging] = useState(false)
  const identityById = useMemo(
    () => new Map(identities.map((identity) => [identity.id, identity])),
    [identities],
  )
  const [fromIdentityId, setFromIdentityId] = useState(
    draft.identityId ||
      identities.find((identity) => identity.primary)?.id ||
      identities[0]?.id ||
      '',
  )
  const [recipientError, setRecipientError] = useState<'to' | 'cc' | 'bcc' | null>(null)
  const dialogRef = useRef<HTMLElement>(null)
  const editorRef = useRef<HTMLDivElement>(null)
  const remoteAutosaveTimerRef = useRef<number | null>(null)
  const initialBodyRef = useRef(draft.body)
  const skipInitialSave = useRef(true)
  useFocusTrap(dialogRef)
  useEffect(() => {
    if (!remote) return
    remoteDraftApi.setActiveId(draft.serverDraftId || null)
    return () => remoteDraftApi.clearActive()
  }, [])
  useEffect(() => {
    if (!remote) return
    let cancelled = false
    void identitiesApi
      .refresh()
      .then((rows) => {
        if (cancelled) return
        setIdentities(rows)
        const selected = draft.identityId
          ? rows.find((identity) => identity.id === draft.identityId)
          : rows.find((identity) => identity.primary) || rows[0]
        if (selected) {
          setFromIdentityId(selected.id)
          setDraft((current) =>
            current.identityId === selected.id
              ? current
              : {
                  ...current,
                  identityId: selected.id,
                  from: { name: selected.displayName, email: selected.email },
                },
          )
        }
      })
      .catch(() => setStatus('Could not load sender identity'))
    return () => {
      cancelled = true
    }
  }, [remote])
  useEffect(() => {
    if (editorRef.current) editorRef.current.textContent = initialBodyRef.current
  }, [])
  const update = (field: keyof Draft, value: string) => {
    setDraft((current) => ({ ...current, [field]: value, sendKey: freshKey() }))
    if (field === 'to' || field === 'cc' || field === 'bcc') setRecipientError(null)
    setStatus('Saving...')
  }
  useEffect(() => {
    if (skipInitialSave.current) {
      skipInitialSave.current = false
      return
    }
    if (remote) return
    const timer = window.setTimeout(() => {
      localStorage.setItem(draftKey, JSON.stringify(draft))
      setStatus('Saved to Drafts')
    }, 400)
    return () => window.clearTimeout(timer)
  }, [draft])
  useEffect(() => {
    if (skipInitialSave.current || !remote) return
    const timer = window.setTimeout(() => {
      remoteAutosaveTimerRef.current = null
      if (
        !draft.to &&
        !draft.cc &&
        !draft.bcc &&
        !draft.subject &&
        !draft.body &&
        draft.attachments.length === 0
      )
        return
      const compose = {
        to: parseRecipients(draft.to),
        cc: parseRecipients(draft.cc),
        bcc: parseRecipients(draft.bcc),
        subject: draft.subject,
        body_text: draft.body,
        attachments: draft.attachments.map((attachment) => ({ id: attachment.id })),
        identity_id: fromIdentityId || draft.identityId,
        client_key: draft.clientKey,
        send_key: draft.sendKey,
      }
      remoteDraftApi
        .save(compose)
        .then(() => {
          if (!sendingRef.current) setStatus('Saved to Drafts')
        })
        .catch(() => {
          if (!sendingRef.current) setStatus('Draft not saved — retrying')
        })
    }, 1500)
    remoteAutosaveTimerRef.current = timer
    return () => {
      window.clearTimeout(timer)
      if (remoteAutosaveTimerRef.current === timer) remoteAutosaveTimerRef.current = null
    }
  }, [draft])
  const syncBody = () => {
    update('body', editorRef.current?.innerText ?? '')
  }
  const format = (command: string) => {
    editorRef.current?.focus()
    document.execCommand(command)
  }
  const toggleSignature = () => {
    if (!editorRef.current || !signatureBlock) return
    const nextBody = includeSignature
      ? draft.body.replace(new RegExp(`${escapeRegExp(signatureBlock)}\\s*$`), '')
      : draft.body + signatureBlock
    setDraft((current) => ({ ...current, body: nextBody, sendKey: freshKey() }))
    editorRef.current.textContent = nextBody
    setIncludeSignature((current) => !current)
    setStatus('Saving...')
  }
  const addFiles = async (files: File[] | null) => {
    if (!files?.length) return
    setUploading(true)
    setStatus('Uploading attachment...')
    try {
      const uploaded: DraftAttachment[] = []
      for (const file of files) uploaded.push(await uploadAttachment(file))
      setDraft((current) => ({
        ...current,
        attachments: [...current.attachments, ...uploaded],
        sendKey: freshKey(),
      }))
      setStatus('Saved to Drafts')
    } catch (error) {
      setStatus(error instanceof Error ? error.message : 'Upload failed')
    } finally {
      setUploading(false)
    }
  }
  const removeAttachment = (attachment: DraftAttachment) => {
    setDraft((current) => ({
      ...current,
      attachments: current.attachments.filter((item) => item.id !== attachment.id),
      sendKey: freshKey(),
    }))
    void deleteStagedAttachment(attachment.id).catch(() => {})
  }
  const onPaste = (event: ClipboardEvent<HTMLDivElement>) => {
    const files = Array.from(event.clipboardData.files)
    if (!files.length) return
    event.preventDefault()
    void addFiles(files)
  }
  const onDrop = (event: DragEvent<HTMLDivElement>) => {
    event.preventDefault()
    setDragging(false)
    void addFiles(Array.from(event.dataTransfer.files))
  }
  const discard = () => {
    const attachmentIds = draft.attachments.map((attachment) => attachment.id)
    const activeDraft = remoteDraftApi.activeId()
    if (isRemoteMail()) {
      void (async () => {
        if (activeDraft) {
          try {
            await remoteDraftApi.remove(activeDraft)
          } catch {
            /* server cleanup worker still protects/reclaims referenced files */
          }
        }
        await Promise.all(attachmentIds.map((id) => deleteStagedAttachment(id).catch(() => {})))
      })()
    }
    draftsApi.clear()
    close()
  }
  const send = async (event: FormEvent) => {
    event.preventDefault()
    if (uploading || sendingRef.current) return
    if (remote && (!fromIdentityId || !identityById.has(fromIdentityId))) {
      setStatus('A verified mailbox sender is required before sending')
      return
    }
    if (!draft.to.trim()) {
      setStatus('Add at least one recipient')
      return
    }
    if (!draft.subject.trim() && !draft.body.trim()) {
      setStatus('Add a subject or message')
      return
    }
    const invalid = (value: string) => parseAddresses(value).some(addressIsInvalid)
    if (invalid(draft.to)) {
      setRecipientError('to')
      setStatus('Check the recipients: enter valid email addresses')
      return
    }
    if (invalid(draft.cc)) {
      setRecipientError('cc')
      setStatus('Check the recipients: enter valid email addresses')
      return
    }
    if (invalid(draft.bcc)) {
      setRecipientError('bcc')
      setStatus('Check the recipients: enter valid email addresses')
      return
    }
    const finalizedDraft = { ...draft, identityId: fromIdentityId || draft.identityId }
    if (remoteAutosaveTimerRef.current !== null) {
      window.clearTimeout(remoteAutosaveTimerRef.current)
      remoteAutosaveTimerRef.current = null
    }
    sendingRef.current = true
    setSending(true)
    try {
      if (remote) {
        setStatus('Saving draft before send...')
        await remoteDraftApi.save({
          to: parseRecipients(finalizedDraft.to),
          cc: parseRecipients(finalizedDraft.cc),
          bcc: parseRecipients(finalizedDraft.bcc),
          subject: finalizedDraft.subject,
          body_text: finalizedDraft.body,
          attachments: finalizedDraft.attachments.map((attachment) => ({ id: attachment.id })),
          identity_id: finalizedDraft.identityId,
          client_key: finalizedDraft.clientKey,
          send_key: finalizedDraft.sendKey,
        })
      }
      setStatus(finalizedDraft.scheduledAt ? 'Scheduling...' : 'Sending...')
      const ok = (await onSent?.(finalizedDraft)) ?? true
      if (!ok) {
        setStatus(
          finalizedDraft.scheduledAt
            ? 'Could not schedule — try again'
            : 'Could not send — check the alert and try again',
        )
        return
      }
      draftsApi.clear()
      setStatus(finalizedDraft.scheduledAt ? 'Scheduled' : 'Message sent')
      window.setTimeout(close, 700)
    } catch (error) {
      setStatus(
        error instanceof Error ? `Could not send — ${error.message}` : 'Could not send — try again',
      )
    } finally {
      sendingRef.current = false
      setSending(false)
    }
  }
  return (
    <div className="compose-layer">
      <datalist id="cs-mail-contacts">
        {contactsService.list().map((contact) => (
          <option key={contact.email} value={`${contact.name} <${contact.email}>`} />
        ))}
      </datalist>
      <section
        ref={dialogRef}
        className="composer"
        role="dialog"
        aria-modal="true"
        aria-label="New message"
      >
        <header>
          <strong>{draft.scheduledAt ? 'Schedule message' : 'New message'}</strong>
          <span>
            <small>{status}</small>
            <button type="button" className="icon-button" onClick={close} aria-label="Close">
              <X size={16} />
            </button>
          </span>
        </header>
        <form onSubmit={send}>
          <label>
            From
            <select
              aria-label="From address"
              value={fromIdentityId}
              onChange={(event) => {
                const id = event.target.value
                setFromIdentityId(id)
                const identity = identityById.get(id)
                if (identity) {
                  setDraft((current) => ({
                    ...current,
                    identityId: identity.id,
                    from: { name: identity.displayName, email: identity.email },
                    sendKey: freshKey(),
                  }))
                }
              }}
            >
              {identities.map((identity) => (
                <option key={identity.id} value={identity.id}>
                  {identity.displayName} &lt;{identity.email}&gt;
                </option>
              ))}
            </select>
          </label>
          <label>
            To
            <input
              list="cs-mail-contacts"
              autoFocus
              value={draft.to}
              onChange={(event) => update('to', event.target.value)}
              placeholder="Recipients"
            />
            <button
              type="button"
              className="field-link"
              onClick={() => setShowCopies((value) => !value)}
            >
              {showCopies ? 'Hide' : 'Cc Bcc'}
            </button>
          </label>
          {recipientError === 'to' && (
            <p className="composer-error">
              Enter a valid email address, or separate recipients with commas
            </p>
          )}
          {showCopies && (
            <>
              <label>
                Cc
                <input
                  list="cs-mail-contacts"
                  value={draft.cc}
                  onChange={(event) => update('cc', event.target.value)}
                  placeholder="Carbon copy"
                />
              </label>
              {recipientError === 'cc' && (
                <p className="composer-error">Enter a valid email address</p>
              )}
              <label>
                Bcc
                <input
                  list="cs-mail-contacts"
                  value={draft.bcc}
                  onChange={(event) => update('bcc', event.target.value)}
                  placeholder="Blind carbon copy"
                />
              </label>
              {recipientError === 'bcc' && (
                <p className="composer-error">Enter a valid email address</p>
              )}
            </>
          )}
          <label>
            Subject
            <input
              value={draft.subject}
              onChange={(event) => update('subject', event.target.value)}
              placeholder="Subject"
            />
          </label>
          <div
            className={`composer-editor ${dragging ? 'composer-editor--drag' : ''}`}
            onDragOver={(event) => event.preventDefault()}
            onDragEnter={(event) => {
              event.preventDefault()
              setDragging(true)
            }}
            onDragLeave={() => setDragging(false)}
            onDrop={onDrop}
          >
            <div className="formatting">
              <button
                type="button"
                aria-label="Bold"
                onMouseDown={(event) => event.preventDefault()}
                onClick={() => format('bold')}
              >
                <strong>B</strong>
              </button>
              <button
                type="button"
                aria-label="Italic"
                onMouseDown={(event) => event.preventDefault()}
                onClick={() => format('italic')}
              >
                <em>I</em>
              </button>
              <button
                type="button"
                aria-label="Underline"
                onMouseDown={(event) => event.preventDefault()}
                onClick={() => format('underline')}
              >
                <u>U</u>
              </button>
              {signatureBlock && (
                <label className="composer-signature-toggle">
                  <input
                    type="checkbox"
                    aria-label="Include signature"
                    checked={includeSignature}
                    onChange={toggleSignature}
                  />
                  Signature
                </label>
              )}
            </div>
            <div
              ref={editorRef}
              className="composer-editor__body"
              contentEditable
              role="textbox"
              aria-multiline="true"
              aria-label="Message body"
              onInput={syncBody}
              onPaste={onPaste}
              suppressContentEditableWarning
            />
            {draft.attachments.length > 0 && (
              <div className="chips">
                {draft.attachments.map((attachment) => (
                  <small className="attachment-chip" key={attachment.id}>
                    <Paperclip size={13} />
                    {attachment.filename}
                    <button
                      type="button"
                      aria-label={`Remove ${attachment.filename}`}
                      onClick={() => removeAttachment(attachment)}
                    >
                      <X size={12} />
                    </button>
                  </small>
                ))}
              </div>
            )}
          </div>
          <footer>
            <label className="attach-control">
              <Paperclip size={17} />
              <input
                type="file"
                multiple
                onChange={(event) => void addFiles(Array.from(event.target.files ?? []))}
                aria-label="Attach a file"
              />
            </label>
            <input
              aria-label="Schedule message"
              type="datetime-local"
              value={draft.scheduledAt}
              onChange={(event) => update('scheduledAt', event.target.value)}
            />
            <button className="secondary-button" type="button" onClick={discard}>
              Discard
            </button>
            <button
              className="primary-button compose-send"
              type="submit"
              disabled={uploading || sending}
              aria-busy={sending}
            >
              <Send size={15} />
              {sending
                ? draft.scheduledAt
                  ? 'Scheduling…'
                  : 'Sending…'
                : draft.scheduledAt
                  ? 'Schedule'
                  : 'Send'}
            </button>
            {draft.scheduledAt && (
              <small className="scheduled-hint">
                <Clock3 size={13} />
                Queued for delivery
              </small>
            )}
          </footer>
        </form>
      </section>
    </div>
  )
}
