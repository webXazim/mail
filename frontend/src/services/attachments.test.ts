import { beforeEach, describe, expect, it, vi } from 'vitest'

const { tokenStore, mailboxContextStore, refreshSession } = vi.hoisted(() => ({
  tokenStore: { getAccess: vi.fn(() => 'access-token') },
  mailboxContextStore: {
    getOrganizationId: vi.fn(() => 'organization-1'),
    getMailboxId: vi.fn(() => 'mailbox-1'),
  },
  refreshSession: vi.fn(),
}))

vi.mock('../lib/api', () => ({ tokenStore, mailboxContextStore, refreshSession }))

import { uploadAttachment } from './attachments'

describe('outgoing attachment upload', () => {
  beforeEach(() => { vi.restoreAllMocks() })

  it('sends the active mailbox context with the file', async () => {
    const send = vi.fn().mockResolvedValue({
      ok: true,
      status: 201,
      json: async () => ({ attachment: { id: 'attachment-1', filename: 'photo.png', size: 3 } }),
    })
    vi.stubGlobal('fetch', send)

    const file = new File(['abc'], 'photo.png', { type: 'image/png' })
    const uploaded = await uploadAttachment(file)

    const [, request] = send.mock.calls[0] as [string, RequestInit]
    const headers = new Headers(request.headers)
    expect(headers.get('Authorization')).toBe('Bearer access-token')
    expect(headers.get('X-CS-Organization-ID')).toBe('organization-1')
    expect(headers.get('X-CS-Mailbox-ID')).toBe('mailbox-1')
    expect(request.body).toBe(file)
    expect(uploaded.id).toBe('attachment-1')
  })
})
