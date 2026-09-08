const apiBase = import.meta.env.VITE_API_URL as string | undefined

const localAttachmentKey = (name: string) => `harbor-mail:attachment:${name}`
const remoteUrlKey = (name: string) => `harbor-mail:attachment-url:${name}`

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

const rememberRemoteUrl = (name: string, url: string) => {
  try {
    localStorage.setItem(remoteUrlKey(name), url)
  } catch {
    /* quota is not available; fall back to file-name resolution */
  }
}

export async function uploadAttachment(file: File) {
  if (!apiBase) {
    await persistLocalAttachment(file)
    return { name: file.name, size: file.size, local: true }
  }
  const body = new FormData()
  body.append('file', file)
  const response = await fetch(`${apiBase}/attachments`, { method: 'POST', credentials: 'include', body })
  if (!response.ok) throw new Error('Attachment upload failed')
  const result = (await response.json()) as { name: string; size: number; url?: string }
  if (result.url) rememberRemoteUrl(result.name, result.url)
  return result
}

export function getLocalAttachment(name: string): Promise<Blob | null> {
  const dataUrl = localStorage.getItem(localAttachmentKey(name))
  if (!dataUrl) return Promise.resolve(null)
  return fetch(dataUrl)
    .then(response => (response.ok ? response.blob() : null))
    .catch(() => null)
}

export function getRemoteAttachmentUrl(name: string): string | null {
  return localStorage.getItem(remoteUrlKey(name))
}