import { describe, expect, it } from 'vitest'
import { buildReplyMail } from './delivery'
import type { Draft } from '../types'

const draft: Draft = { to: 'Nora Li <nora@northstar.studio>', cc: '', bcc: '', subject: 'Launch plan', body: 'Hi', attachments: [], scheduledAt: '', from: { name: 'Alex Morgan', email: 'alex@harbor.co' } }

describe('buildReplyMail', () => {
  it('replies to the first recipient with a Re: subject', () => {
    const reply = buildReplyMail(draft, 'Nora Li')
    expect(reply.email).toBe('nora@northstar.studio')
    expect(reply.sender).toBe('Nora Li')
    expect(reply.subject).toBe('Re: Launch plan')
    expect(reply.unread).toBe(true)
    expect(reply.label).toBe('Inbox')
  })

  it('derives an uppercase display name when none is known', () => {
    const reply = buildReplyMail({ ...draft, to: 'riley@papertrail.com' })
    expect(reply.sender).toBe('Riley')
    expect(reply.email).toBe('riley@papertrail.com')
  })

  it('falls back gracefully without a subject or recipients', () => {
    const reply = buildReplyMail({ ...draft, to: '', subject: '' })
    expect(reply.subject).toBe('Re: your message')
  })
})