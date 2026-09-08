const apiBase = import.meta.env.VITE_API_URL as string | undefined
export async function uploadAttachment(file: File) {
  if (!apiBase) return { name: file.name, size: file.size, local: true }
  const body = new FormData(); body.append('file', file)
  const response = await fetch(`${apiBase}/attachments`, { method: 'POST', credentials: 'include', body }); if (!response.ok) throw new Error('Attachment upload failed'); return response.json() as Promise<{ name: string; size: number; url?: string }>
}
