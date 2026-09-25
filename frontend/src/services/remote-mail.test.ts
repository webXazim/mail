import { describe, expect, it } from 'vitest'
import { rawThreadToItem } from './remote-mail'

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
      attachments: [{ name: 'invoice.pdf', blobId: 'blob-1', type: 'application/pdf' }],
    })

    expect(item.threadId).toBe('eaaaab')
    expect(item.sender).toBe('Webx Azim')
    expect(item.email).toBe('webxazim@gmail.com')
    expect(item.clearBody).toBe('The actual message body')
    expect(item.to).toEqual(['hello@webxazim.com'])
    expect(item.attachments?.[0].blobId).toBe('blob-1')
  })
})
