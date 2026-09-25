import { beforeEach, describe, expect, it, vi } from 'vitest'

const { apiFetch } = vi.hoisted(() => ({ apiFetch: vi.fn() }))

vi.mock('../lib/api', () => ({ apiFetch }))

import { composeToDraft, rawThreadToItem, remoteDraftApi } from './remote-mail'

describe('live thread response mapping', () => {
  it('maps a Stalwart Email/get message to a real reader item', () => {
    const item = rawThreadToItem({
      id: 'email-1',
      threadId: 'eaaaab',
      receivedAt: new Date().toISOString(),
      from: [{ name: 'Webx Azim', email: 'webxazim@gmail.com' }],
      to: [{ email: 'hello@webxazim.com' }],
      subject: 'Test',
      preview: 'Hello world',
      body_text: 'The actual message body',
      'header:Message-ID': '<original@example.com>',
      'header:References': '<first@example.com> <prior@example.com>',
      attachments: [{ name: 'invoice.pdf', blobId: 'blob-1', type: 'application/pdf' }],
    })

    expect(item.threadId).toBe('eaaaab')
    expect(item.sender).toBe('Webx Azim')
    expect(item.email).toBe('webxazim@gmail.com')
    expect(item.clearBody).toBe('The actual message body')
    expect(item.to).toEqual(['hello@webxazim.com'])
    expect(item.attachments?.[0].blobId).toBe('blob-1')
    expect(item.messageId).toBe('<original@example.com>')
    expect(item.references).toEqual(['<first@example.com>', '<prior@example.com>'])
  })

  it('keeps threading headers when reopening a saved reply draft', () => {
    const draft = composeToDraft({
      to: [{ email: 'person@example.com' }], cc: [], bcc: [],
      subject: 'Re: Hello', body_text: 'Reply', attachments: [],
      in_reply_to: '<original@example.com>',
      references: ['<first@example.com>', '<original@example.com>'],
    })
    expect(draft.inReplyTo).toBe('<original@example.com>')
    expect(draft.references).toEqual(['<first@example.com>', '<original@example.com>'])
  })
})

describe('remote draft saves', () => {
  beforeEach(() => {
    apiFetch.mockReset()
    remoteDraftApi.clearActive()
  })

  it('serializes autosave and send-time save into one draft', async () => {
    let finishCreate: ((value: { id: string }) => void) | undefined
    apiFetch.mockImplementationOnce(
      () =>
        new Promise<{ id: string }>((resolve) => {
          finishCreate = resolve
        }),
    )
    apiFetch.mockResolvedValueOnce({})

    const compose = {
      to: [{ email: 'recipient@example.com' }],
      cc: [],
      bcc: [],
      subject: 'Hello',
      body_text: 'Message',
      attachments: [],
    }
    const autosave = remoteDraftApi.save(compose)
    const sendTimeSave = remoteDraftApi.save(compose)

    await Promise.resolve()
    expect(apiFetch).toHaveBeenCalledTimes(1)
    expect(apiFetch).toHaveBeenNthCalledWith(
      1,
      '/api/drafts',
      expect.objectContaining({ method: 'POST' }),
    )

    finishCreate?.({ id: 'draft-1' })
    await expect(autosave).resolves.toBe('draft-1')
    await expect(sendTimeSave).resolves.toBe('draft-1')
    expect(apiFetch).toHaveBeenCalledTimes(2)
    expect(apiFetch).toHaveBeenNthCalledWith(
      2,
      '/api/drafts/draft-1',
      expect.objectContaining({ method: 'PUT' }),
    )
  })
})
