import { ApiError, apiFetch, mailboxContextStore, refreshSession, tokenStore } from '../lib/api'

export type AppPassword = {
  id: string
  label: string
  allowed_ips: string[]
  expires_at: string | null
  status: string
  last_error: string
  created_at: string
  revoked_at: string | null
}

export type MailEndpoint = {
  protocol: 'IMAP' | 'SMTP'
  host: string
  port: number
  security: 'TLS' | 'STARTTLS'
  authentication: 'app_password'
}

export type MailClientConfig = {
  username: string
  incoming: MailEndpoint
  outgoing: MailEndpoint
}

export type MailClientOverview = {
  mailbox: { id: string; address: string; organization_id: string }
  config: MailClientConfig
  max_app_passwords: number
  app_passwords: AppPassword[]
  autoconfig: { thunderbird_path: string; note: string }
}

export type CreatedAppPassword = {
  id: string
  label: string
  secret: string
  expires_at: string | null
  config: MailClientConfig
  message: string
}

export type MailImport = {
  id: string
  original_filename: string
  byte_size: number
  status: string
  total_messages: number
  imported_messages: number
  failed_messages: number
  last_error: string
  attempts: number
  max_attempts: number
  started_at: string | null
  completed_at: string | null
  created_at: string
  updated_at: string
}

async function parseUploadResponse<T>(response: Response): Promise<T> {
  const text = await response.text()
  let body: unknown = null
  if (text) {
    try { body = JSON.parse(text) } catch { body = text }
  }
  if (!response.ok) {
    const message = body && typeof body === 'object' && 'message' in body
      ? String((body as Record<string, unknown>).message)
      : `Request failed (${response.status})`
    const code = body && typeof body === 'object' && 'error' in body
      ? String((body as Record<string, unknown>).error)
      : 'api_error'
    throw new ApiError(response.status, message, code, response.headers.get('x-request-id'))
  }
  return body as T
}

async function uploadRaw(file: File, retry = true): Promise<{ id: string; status: string; filename: string; byte_size: number }> {
  const headers = new Headers({ 'Content-Type': file.type || 'application/mbox' })
  const access = tokenStore.getAccess()
  if (access) headers.set('Authorization', `Bearer ${access}`)
  const organizationId = mailboxContextStore.getOrganizationId()
  const mailboxId = mailboxContextStore.getMailboxId()
  if (organizationId) headers.set('X-CS-Organization-ID', organizationId)
  if (mailboxId) headers.set('X-CS-Mailbox-ID', mailboxId)
  const response = await fetch(`/api/mail-imports?filename=${encodeURIComponent(file.name)}`, {
    method: 'POST',
    headers,
    credentials: 'include',
    body: file,
  })
  if (response.status === 401 && retry && await refreshSession()) return uploadRaw(file, false)
  return parseUploadResponse<{ id: string; status: string; filename: string; byte_size: number }>(response)
}

export const mailClientsApi = {
  overview: () => apiFetch<MailClientOverview>('/mail-clients'),
  createAppPassword: (input: {
    label: string
    current_password: string
    expires_at?: string | null
    allowed_ips?: string[]
  }) => apiFetch<CreatedAppPassword>('/mail-clients/app-passwords', {
    method: 'POST',
    body: JSON.stringify(input),
  }),
  revokeAppPassword: (id: string) => apiFetch<{ ok: boolean }>(`/mail-clients/app-passwords/${id}`, { method: 'DELETE' }),
  imports: async () => (await apiFetch<{ imports: MailImport[] }>('/mail-imports')).imports,
  uploadImport: uploadRaw,
  cancelImport: (id: string) => apiFetch<{ ok: boolean }>(`/mail-imports/${id}/cancel`, { method: 'POST' }),
}
