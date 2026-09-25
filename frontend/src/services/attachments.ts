import { ApiError, mailboxContextStore, refreshSession, tokenStore } from '../lib/api'
import type { DraftAttachment } from '../types'

const localBlobs = new Map<string, Blob>()
const localNames = new Map<string, string>()

const localId = () =>
  typeof crypto !== 'undefined' && 'randomUUID' in crypto
    ? `local-${crypto.randomUUID()}`
    : `local-${Date.now()}-${Math.random().toString(16).slice(2)}`

const parseError = async (response: Response) => {
  let message = `Upload failed (${response.status})`
  let code = 'attachment_upload_failed'
  try {
    const body = (await response.json()) as { message?: string; error?: string }
    if (body.message) message = body.message
    if (body.error) code = body.error
  } catch {
    /* non-JSON provider/proxy failure */
  }
  throw new ApiError(response.status, message, code, response.headers.get('x-request-id'))
}

const attachmentHeaders = (contentType?: string): Headers => {
  const headers = new Headers()
  if (contentType) headers.set('Content-Type', contentType)
  const access = tokenStore.getAccess()
  if (access) headers.set('Authorization', `Bearer ${access}`)
  const organizationId = mailboxContextStore.getOrganizationId()
  const mailboxId = mailboxContextStore.getMailboxId()
  if (organizationId) headers.set('X-CS-Organization-ID', organizationId)
  if (mailboxId) headers.set('X-CS-Mailbox-ID', mailboxId)
  return headers
}

async function remoteUpload(file: File): Promise<DraftAttachment> {
  const send = async () => {
    return fetch(`/api/attachments?filename=${encodeURIComponent(file.name)}`, {
      method: 'POST',
      headers: attachmentHeaders(file.type || 'application/octet-stream'),
      body: file,
      credentials: 'include',
    })
  }

  let response = await send()
  if (response.status === 401 && (await refreshSession())) response = await send()
  if (!response.ok) return parseError(response)
  const data = (await response.json()) as { attachment: DraftAttachment }
  return data.attachment
}

/**
 * Upload a compose attachment. Authenticated sessions stream bytes directly to
 * the API staging volume; demo mode keeps the Blob only in memory.
 */
export async function uploadAttachment(file: File): Promise<DraftAttachment> {
  if (tokenStore.getAccess()) return remoteUpload(file)

  const id = localId()
  localBlobs.set(id, file)
  localNames.set(file.name, id)
  return {
    id,
    filename: file.name,
    content_type: file.type || 'application/octet-stream',
    size: file.size,
    status: 'ready',
  }
}

/** Demo/local reader helper. No attachment bytes are persisted in localStorage. */
export function getLocalAttachment(idOrName: string): Promise<Blob | null> {
  const id = localBlobs.has(idOrName) ? idOrName : localNames.get(idOrName)
  return Promise.resolve(id ? localBlobs.get(id) ?? null : null)
}

/** Fetch a received-message Stalwart blob over the authenticated mailbox API. */
export async function getRemoteAttachment(blobId: string): Promise<Blob | null> {
  try {
    const response = await fetch(`/api/mail/attachment/${encodeURIComponent(blobId)}`, {
      headers: attachmentHeaders(),
      credentials: 'include',
    })
    return response.ok ? await response.blob() : null
  } catch {
    return null
  }
}

/** Fetch a staged outbound blob owned by the current account. */
export async function getStagedAttachment(id: string): Promise<Blob | null> {
  if (id.startsWith('local-')) return getLocalAttachment(id)
  try {
    const response = await fetch(`/api/attachments/${encodeURIComponent(id)}`, {
      headers: attachmentHeaders(),
      credentials: 'include',
    })
    return response.ok ? await response.blob() : null
  } catch {
    return null
  }
}

/** Best-effort release of an unreferenced staged upload. Referenced blobs are
 * intentionally kept by the backend until their draft/schedule is updated. */
export async function deleteStagedAttachment(id: string): Promise<void> {
  if (id.startsWith('local-')) {
    localBlobs.delete(id)
    for (const [name, value] of localNames) if (value === id) localNames.delete(name)
    return
  }

  const send = async () => {
    return fetch(`/api/attachments/${encodeURIComponent(id)}`, {
      method: 'DELETE',
      headers: attachmentHeaders(),
      credentials: 'include',
    })
  }
  let response = await send()
  if (response.status === 401 && (await refreshSession())) response = await send()
  // 409 means a draft/scheduled row still references it; backend lifecycle
  // cleanup will reclaim it after that reference is removed.
  if (!response.ok && response.status !== 404 && response.status !== 409) await parseError(response)
}
