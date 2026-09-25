import { describe, expect, it } from 'vitest'
import { buildReplyAllDraft, buildReplyDraft, conversationReplyRecipients } from './mail'
import type { Mail, ReaderThreadItem } from '../types'

const mail: Mail = {
  id: 'maaaaad', initials: 'WA', sender: 'Webx Azim', email: 'webxazim@gmail.com',
  subject: 'Test', preview: 'Hello', time: 'Today', label: 'Mail', color: 'teal',
  unread: false, to: ['hello@webxazim.com'], folder: 'Inbox',
}

const source: ReaderThreadItem = {
  id: 'maaaaad', threadId: 'eaaaac', initials: 'WA', sender: 'Webx Azim',
  email: 'webxazim@gmail.com', subject: 'Test', copy: 'Hello', clearBody: 'Hello',
  bodyHtml: '', time: 'Today', color: 'teal',
  messageId: '<original@example.com>', references: ['<earlier@example.com>'],
}

describe('reply conversation headers', () => {
  it('links a reply to the source message', () => {
    expect(buildReplyDraft(mail, source)).toMatchObject({
      inReplyTo: '<original@example.com>',
      references: ['<earlier@example.com>', '<original@example.com>'],
    })
  })

  it('links reply-all without requiring a source header for demo messages', () => {
    expect(buildReplyAllDraft(mail, 'hello@webxazim.com', source).inReplyTo)
      .toBe('<original@example.com>')
    expect(buildReplyDraft(mail).inReplyTo).toBeUndefined()
  })
})

describe('conversation reply recipients', () => {
  const own = 'hello@webxazim.com'
  const incoming = { ...source, email: 'msecure6666@gmail.com' }
  const sentToSelf: ReaderThreadItem = {
    ...source,
    id: 'sent-self',
    sender: 'Hello',
    email: own,
    to: [own],
  }

  it('uses the external correspondent when replying to a self-addressed sent copy', () => {
    expect(conversationReplyRecipients(sentToSelf, [incoming, sentToSelf], [own])).toEqual({
      to: ['msecure6666@gmail.com'], cc: [],
    })
  })

  it('replies to recipients rather than the sender of an outgoing message', () => {
    const sent = { ...sentToSelf, to: ['M Secure <msecure6666@gmail.com>'] }
    expect(conversationReplyRecipients(sent, [incoming, sent], [own])).toEqual({
      to: ['M Secure <msecure6666@gmail.com>'], cc: [],
    })
  })

  it('excludes own identities and duplicate recipients from reply all', () => {
    const received = {
      ...incoming,
      to: [own, 'Other <other@example.com>'],
      cc: ['Other <other@example.com>', 'third@example.com'],
    }
    expect(conversationReplyRecipients(received, [received], [own], true)).toEqual({
      to: ['msecure6666@gmail.com', 'Other <other@example.com>'],
      cc: ['third@example.com'],
    })
  })
})
