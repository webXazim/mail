import { describe, expect, it } from 'vitest'
import { messages } from '../../data'
import type { Mail } from '../../types'
import { initialMailState, mailboxReducer, type MailState } from './mailboxReducer'

const seed = (mailbox: Mail[] = messages): MailState => ({ ...initialMailState, loading: false, mailbox })

describe('mailboxReducer', () => {
  it('hydrates the mailbox and exits loading', () => {
    const next = mailboxReducer(initialMailState, { type: 'hydrated', mailbox: messages })
    expect(next.mailbox).toBe(messages)
    expect(next.loading).toBe(false)
  })

  it('reports a load failure and stops loading', () => {
    const next = mailboxReducer(initialMailState, { type: 'load-failed' })
    expect(next.loading).toBe(false)
    expect(next.notice).toBe('Unable to sync mailbox')
  })

  it('archives the requested ids and records an undo snapshot', () => {
    const next = mailboxReducer(seed(), { type: 'apply', ids: ['m1', 'm2'], action: 'archive' })
    expect(next.mailbox.find(mail => mail.id === 'm1')?.folder).toBe('Archive')
    expect(next.mailbox.find(mail => mail.id === 'm2')?.folder).toBe('Archive')
    expect(next.mailbox.find(mail => mail.id === 'm3')?.folder).toBe('Inbox')
    expect(next.undo).toEqual({ previous: messages, ids: ['m1', 'm2'] })
    expect(next.notice).toBe('Archived')
  })

  it('moves messages to trash', () => {
    const next = mailboxReducer(seed(), { type: 'apply', ids: ['m1'], action: 'trash' })
    expect(next.mailbox.find(mail => mail.id === 'm1')?.folder).toBe('Trash')
    expect(next.notice).toBe('Moved to Trash')
  })

  it('marks messages as read', () => {
    const next = mailboxReducer(seed(), { type: 'apply', ids: ['m1'], action: 'read' })
    expect(next.mailbox.find(mail => mail.id === 'm1')?.unread).toBe(false)
    expect(next.undo).not.toBeNull()
  })

  it('does not mutate or snapshot when no ids are selected', () => {
    const previous = seed()
    const next = mailboxReducer(previous, { type: 'apply', ids: [], action: 'archive' })
    expect(next.mailbox).toBe(previous.mailbox)
    expect(next.undo).toBeNull()
    expect(next.notice).toBe('Select at least one message first')
  })

  it('undo restores the previous mailbox and clears the snapshot', () => {
    const applied = mailboxReducer(seed(), { type: 'apply', ids: ['m1'], action: 'trash' })
    const next = mailboxReducer(applied, { type: 'undo' })
    expect(next.mailbox).toBe(messages)
    expect(next.undo).toBeNull()
    expect(next.notice).toBe('Action undone')
  })

  it('too-late clears the undo snapshot without restoring the mailbox', () => {
    const applied = mailboxReducer(seed(), { type: 'apply', ids: ['m1'], action: 'trash' })
    const next = mailboxReducer(applied, { type: 'too-late' })
    expect(next.mailbox.find(mail => mail.id === 'm1')?.folder).toBe('Trash')
    expect(next.undo).toBeNull()
  })

  it('mark-read silences unread state without an undo snapshot or notice', () => {
    const next = mailboxReducer(seed(), { type: 'mark-read', ids: ['m1', 'm3'] })
    expect(next.mailbox.find(mail => mail.id === 'm1')?.unread).toBe(false)
    expect(next.mailbox.find(mail => mail.id === 'm3')?.unread).toBe(false)
    expect(next.undo).toBeNull()
    expect(next.notice).toBe('')
  })

  it('toggle-star flips the starred flag with an undo snapshot', () => {
    const next = mailboxReducer(seed(), { type: 'toggle-star', ids: ['m1'] })
    expect(next.mailbox.find(mail => mail.id === 'm1')?.starred).toBe(false)
    expect(next.undo?.ids).toEqual(['m1'])
    const back = mailboxReducer(next, { type: 'toggle-star', ids: ['m1'] })
    expect(back.mailbox.find(mail => mail.id === 'm1')?.starred).toBe(true)
  })

  it('toggles a label on and off for the requested ids', () => {
    const applied = mailboxReducer(seed(), { type: 'toggle-label', ids: ['m1'], label: 'Internal' })
    expect(applied.mailbox.find(mail => mail.id === 'm1')?.label).toBe('Internal')
    const cleared = mailboxReducer(applied, { type: 'toggle-label', ids: ['m1'], label: 'Internal' })
    expect(cleared.mailbox.find(mail => mail.id === 'm1')?.label).toBe('')
  })

  it('marks messages as unread', () => {
    const next = mailboxReducer(seed(), { type: 'mark-unread', ids: ['m1', 'm3'] })
    expect(next.mailbox.find(mail => mail.id === 'm1')?.unread).toBe(true)
    expect(next.mailbox.find(mail => mail.id === 'm3')?.unread).toBe(true)
  })

  it('moves messages to another folder with a notice', () => {
    const next = mailboxReducer(seed(), { type: 'move-to', ids: ['m1'], folder: 'Archive' })
    expect(next.mailbox.find(mail => mail.id === 'm1')?.folder).toBe('Archive')
    expect(next.notice).toBe('Moved to Archive')
  })

  it('snoozes messages into the Snoozed folder with an expiry', () => {
    const next = mailboxReducer(seed(), { type: 'snooze', ids: ['m1'], until: new Date('2030-01-01T08:00:00Z').toISOString() })
    const snoozed = next.mailbox.find(mail => mail.id === 'm1')
    expect(snoozed?.folder).toBe('Snoozed')
    expect(snoozed?.snoozedUntil).toBeDefined()
    expect(next.notice).toContain('Snoozed until')
  })

  it('unsnooze returns expired messages to the Inbox and keeps future ones', () => {
    const expired: Mail = { ...messages[0], id: 'm-expired', folder: 'Snoozed', snoozedUntil: new Date(Date.now() - 1000).toISOString() }
    const future: Mail = { ...messages[1], id: 'm-future', folder: 'Snoozed', snoozedUntil: new Date(Date.now() + 60_000).toISOString() }
    const state = { ...seed(), mailbox: [expired, future, ...messages.slice(2)] }
    const next = mailboxReducer(state, { type: 'unsnooze' })
    expect(next.mailbox.find(mail => mail.id === 'm-expired')?.folder).toBe('Inbox')
    expect(next.mailbox.find(mail => mail.id === 'm-expired')?.snoozedUntil).toBeUndefined()
    expect(next.mailbox.find(mail => mail.id === 'm-future')?.folder).toBe('Snoozed')
  })

  it('unsends a just-sent message', () => {
    const next = mailboxReducer(seed(), { type: 'unsend-mail', id: 'm1' })
    expect(next.mailbox.some(mail => mail.id === 'm1')).toBe(false)
    expect(next.notice).toBe('Send undone')
  })

  it('prepends a sent message', () => {
    const sent: Mail = { id: 'local-1', initials: 'AM', sender: 'Alex Morgan', email: 'alex@harbor.co', subject: 'Hello', preview: 'Hello!', time: 'Just now', label: 'Sent', color: 'teal', unread: false, folder: 'Sent' }
    const next = mailboxReducer(seed(), { type: 'sent', mail: sent })
    expect(next.mailbox[0].id).toBe('local-1')
    expect(next.notice).toBe('Message sent')
  })

  it('manages transient notice messages', () => {
    const withNotice = mailboxReducer(seed(), { type: 'notice', message: 'Saved' })
    expect(withNotice.notice).toBe('Saved')
    const cleared = mailboxReducer(withNotice, { type: 'clear-notice' })
    expect(cleared.notice).toBe('')
  })
})