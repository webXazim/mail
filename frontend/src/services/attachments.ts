import { tokenStore } from '../lib/api'

const localAttachmentKey = (name: string) => `harbor-mail:attachment:${name}`

const toDataUrl = (file: File) =>
  new Promise<string>((resolve, reject) => {
    const reader = new FileReader()
    reader.onload = () => resolve(String(reader.result))
    reader.onerror = () => reject(reader.error ?? new Error('Unable to read file'))
    reader.readAsDataURL(file)
  })

const persistLocalAttachment = async (file: File) => {
  try {
    localStorage.setItem(localAttachmentKey(file.name), await toDataUrl(file))
  } catch {
    /* data may exceed the local storage quota; upload still succeeds for this session */
  }
}

export async function uploadAttachment(file: File) {
  await persistLocalAttachment(file)
  return { name: file.name, size: file.size, local: true }
}

export function getLocalAttachment(name: string): Promise<Blob | null> {
  const dataUrl = localStorage.getItem(localAttachmentKey(name))
  if (!dataUrl) return Promise.resolve(null)
  return fetch(dataUrl)
    .then((response) => (response.ok ? response.blob() : null))
    .catch(() => null)
}

/** Fetch a stored mail attachment blob over the authenticated API. */
export async function getRemoteAttachment(blobId: string): Promise<Blob | null> {
  try {
    const access = tokenStore.getAccess()
    const headers = access ? { Authorization: `Bearer ${access}` } : undefined
    const response = await fetch(`/api/mail/attachment/${encodeURIComponent(blobId)}`, { headers })
    return response.ok ? await response.blob() : null
  } catch {
    return null
  }
}

export type AttachmentPayload = {
  filename: string
  content_type: string
  data_base64: string
}

/** The multipart payload for `/api/send` — pulls bytes we stashed on upload. */
export function getAttachmentPayload(name: string): AttachmentPayload | null {
  const dataUrl = localStorage.getItem(localAttachmentKey(name))
  if (!dataUrl) return null
  const comma = dataUrl.indexOf(',')
  if (comma < 0) return null
  const meta = dataUrl.slice(0, comma)
  const mime = meta.match(/^data:([^;]+)/)?.[1] || 'application/octet-stream'
  return { filename: name, content_type: mime, data_base64: dataUrl.slice(comma + 1) }
}
