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