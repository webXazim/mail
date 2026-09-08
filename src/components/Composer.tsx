import { useEffect, useMemo, useRef, useState, type ClipboardEvent, type DragEvent, type FormEvent } from 'react'
import { Clock3, Paperclip, Send, X } from 'lucide-react'
import { useFocusTrap } from '../hooks/useFocusTrap'
import { uploadAttachment } from '../services/attachments'
import { contactsService } from '../services/contacts'
import { draftKey, draftsApi } from '../services/drafts'
import { identitiesApi } from '../services/identities'
import { parseAddresses } from '../lib/mail'
import { settingsApi } from '../services/settings'
import type { Draft } from '../types'

type ComposerProps = { close: () => void; onSent?: (draft: Draft) => void; initialDraft?: Partial<Draft> }
const emptyDraft: Draft = { to: '', cc: '', bcc: '', subject: '', body: '', attachments: [], scheduledAt: '' }

const addressIsInvalid = (part: string) => {
  const addr = part.includes('<') ? (part.match(/<([^>]+)>/)?.[1] ?? part) : part
  return !/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(addr)
}

export function Composer({ close, onSent, initialDraft }: ComposerProps) {
  const identities = useMemo(() => {
    const displayName = settingsApi.load().displayName || 'Alex Morgan'
    return identitiesApi.list().map(identity =>
      identity.primary ? { name: displayName, email: identity.email } : { name: identity.displayName, email: identity.email },
    )
  }, [])
  const [draft, setDraft] = useState<Draft>(() => {
    if (initialDraft) return { ...emptyDraft, ...initialDraft, attachments: initialDraft.attachments ?? [] }
    let saved: Partial<Draft> = {}
    try { saved = JSON.parse(localStorage.getItem(draftKey) || '{}') } catch { /* ignore a malformed autosave */ }
    const signature = settingsApi.load().signature
    if (signature && !saved.body) saved.body = signature
    return { ...emptyDraft, ...saved, attachments: saved.attachments ?? [] }
  })
  const [showCopies, setShowCopies] = useState(Boolean(draft.cc || draft.bcc))
  const [status, setStatus] = useState('Saved to Drafts')
  const [uploading, setUploading] = useState(false)
  const [dragging, setDragging] = useState(false)
  const identityByEmail = useMemo(() => new Map(identities.map(identity => [identity.email, identity])), [identities])
  const [fromIdentity, setFromIdentity] = useState(draft.from?.email ?? identities[0]?.email ?? 'alex@harbor.co')
  const [recipientError, setRecipientError] = useState<'to' | 'cc' | 'bcc' | null>(null)
  const dialogRef = useRef<HTMLElement>(null)
  const editorRef = useRef<HTMLDivElement>(null)
  const initialBodyRef = useRef(draft.body)
  const skipInitialSave = useRef(true)
  useFocusTrap(dialogRef)
  useEffect(() => {
    if (editorRef.current) editorRef.current.textContent = initialBodyRef.current
  }, [])
  const update = (field: keyof Draft, value: string) => { setDraft(current => ({ ...current, [field]: value })); if (field === 'to' || field === 'cc' || field === 'bcc') setRecipientError(null); setStatus('Saving...') }
  useEffect(() => { if (skipInitialSave.current) { skipInitialSave.current = false; return } const timer = window.setTimeout(() => { localStorage.setItem(draftKey, JSON.stringify(draft)); setStatus('Saved to Drafts') }, 400); return () => window.clearTimeout(timer) }, [draft])
  const syncBody = () => { update('body', editorRef.current?.innerText ?? '') }
  const format = (command: string) => { editorRef.current?.focus(); document.execCommand(command) }
  const addFiles = async (files: File[] | null) => {
    if (!files?.length) return
    setUploading(true); setStatus('Uploading attachment...')
    try {
      const names: string[] = []
      for (const file of files) names.push((await uploadAttachment(file)).name)
      setDraft(current => ({ ...current, attachments: [...current.attachments, ...names] }))
      setStatus('Saved to Drafts')
    } catch (error) { setStatus(error instanceof Error ? error.message : 'Upload failed') } finally { setUploading(false) }
  }
  const removeAttachment = (name: string) => setDraft(current => ({ ...current, attachments: current.attachments.filter(item => item !== name) }))
  const onPaste = (event: ClipboardEvent<HTMLDivElement>) => {
    const files = Array.from(event.clipboardData.files)
    if (!files.length) return
    event.preventDefault()
    void addFiles(files)
  }
  const onDrop = (event: DragEvent<HTMLDivElement>) => { event.preventDefault(); setDragging(false); void addFiles(Array.from(event.dataTransfer.files)) }
  const discard = () => { draftsApi.clear(); close() }
  const send = (event: FormEvent) => { event.preventDefault(); if (uploading) return; if (!draft.to.trim()) { setStatus('Add at least one recipient'); return } if (!draft.subject.trim() && !draft.body.trim()) { setStatus('Add a subject or message'); return } const invalid = (value: string) => parseAddresses(value).some(addressIsInvalid); if (invalid(draft.to)) { setRecipientError('to'); setStatus('Check the recipients: enter valid email addresses'); return } if (invalid(draft.cc)) { setRecipientError('cc'); setStatus('Check the recipients: enter valid email addresses'); return } if (invalid(draft.bcc)) { setRecipientError('bcc'); setStatus('Check the recipients: enter valid email addresses'); return } onSent?.(draft); draftsApi.clear(); setStatus(draft.scheduledAt ? 'Scheduled' : 'Message sent'); window.setTimeout(close, 700) }
  return <div className="compose-layer"><datalist id="harbor-contacts">{contactsService.list().map(contact => <option key={contact.email} value={`${contact.name} <${contact.email}>`} />)}</datalist><section ref={dialogRef} className="composer" role="dialog" aria-modal="true" aria-label="New message"><header><strong>{draft.scheduledAt ? 'Schedule message' : 'New message'}</strong><span><small>{status}</small><button type="button" className="icon-button" onClick={close} aria-label="Close"><X size={16}/></button></span></header><form onSubmit={send}><label>From<select aria-label="From address" value={fromIdentity} onChange={event => { const email = event.target.value; setFromIdentity(email); const identity = identityByEmail.get(email); if (identity) setDraft(current => ({ ...current, from: identity })) }}>{identities.map(identity => <option key={identity.email} value={identity.email}>{identity.name} &lt;{identity.email}&gt;</option>)}</select></label><label>To<input list="harbor-contacts" autoFocus value={draft.to} onChange={event => update('to', event.target.value)} placeholder="Recipients"/><button type="button" className="field-link" onClick={() => setShowCopies(value => !value)}>{showCopies ? 'Hide' : 'Cc Bcc'}</button></label>{recipientError === 'to' && <p className="composer-error">Enter a valid email address, or separate recipients with commas</p>}{showCopies && <><label>Cc<input list="harbor-contacts" value={draft.cc} onChange={event => update('cc', event.target.value)} placeholder="Carbon copy"/></label>{recipientError === 'cc' && <p className="composer-error">Enter a valid email address</p>}<label>Bcc<input list="harbor-contacts" value={draft.bcc} onChange={event => update('bcc', event.target.value)} placeholder="Blind carbon copy"/></label>{recipientError === 'bcc' && <p className="composer-error">Enter a valid email address</p>}</>}<label>Subject<input value={draft.subject} onChange={event => update('subject', event.target.value)} placeholder="Subject"/></label><div className={`composer-editor ${dragging ? 'composer-editor--drag' : ''}`} onDragOver={event => event.preventDefault()} onDragEnter={event => { event.preventDefault(); setDragging(true) }} onDragLeave={() => setDragging(false)} onDrop={onDrop}><div className="formatting"><button type="button" aria-label="Bold" onMouseDown={event => event.preventDefault()} onClick={() => format('bold')}><strong>B</strong></button><button type="button" aria-label="Italic" onMouseDown={event => event.preventDefault()} onClick={() => format('italic')}><em>I</em></button><button type="button" aria-label="Underline" onMouseDown={event => event.preventDefault()} onClick={() => format('underline')}><u>U</u></button></div><div ref={editorRef} className="composer-editor__body" contentEditable role="textbox" aria-multiline="true" aria-label="Message body" onInput={syncBody} onPaste={onPaste} suppressContentEditableWarning />{draft.attachments.length > 0 && <div className="chips">{draft.attachments.map(name => <small className="attachment-chip" key={name}><Paperclip size={13}/>{name}<button type="button" aria-label={`Remove ${name}`} onClick={() => removeAttachment(name)}><X size={12}/></button></small>)}</div>}</div><footer><label className="attach-control"><Paperclip size={17}/><input type="file" multiple onChange={event => void addFiles(Array.from(event.target.files ?? []))} aria-label="Attach a file"/></label><input aria-label="Schedule message" type="datetime-local" value={draft.scheduledAt} onChange={event => update('scheduledAt', event.target.value)}/><button className="secondary-button" type="button" onClick={discard}>Discard</button><button className="primary-button compose-send" type="submit"><Send size={15}/>{draft.scheduledAt ? 'Schedule' : 'Send'}</button>{draft.scheduledAt && <small className="scheduled-hint"><Clock3 size={13}/>Queued for delivery</small>}</footer></form></section></div>
}