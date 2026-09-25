import { describe, expect, it } from 'vitest'
import { buildReplyAllDraft, buildReplyDraft } from './mail'
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
