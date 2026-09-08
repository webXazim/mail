import { beforeEach, expect, it } from 'vitest'
import type { Mail } from '../types'
import { applyIncomingFilters } from './pipeline'

beforeEach(() => localStorage.clear())

const mailbox: Mail[] = [
  { id: 'a', initials: 'NS', sender: 'News', email: 'news@harbor.co', subject: 'Digest', preview: 'hi', time: '9:00 AM', label: 'Finance', color: 'coral', unread: true, folder: 'Inbox' },
  { id: 'b', initials: 'BB', sender: 'Bot', email: 'bot@example.com', subject: 'Hello', preview: 'buy', time: '8:00 AM', label: '', color: 'teal', unread: true, folder: 'Inbox' },
  { id: 'c', initials: 'TR', sender: 'Trusted', email: 'trusted@example.com', subject: 'Hi', preview: 'hello', time: '7:00 AM', label: '', color: 'blue', unread: false, folder: 'Spam' },
  { id: 'd', initials: 'AM', sender: 'Alex', email: 'alex@harbor.co', subject: 'Sent', preview: 'done', time: '6:00 AM', label: 'Sent', color: 'teal', unread: false, folder: 'Sent' },
]

it('moves blocked senders to Spam and lets allowed senders out of Spam', () => {
  localStorage.setItem('harbor-mail:spam', JSON.stringify({ blocked: ['bot@example.com'], allowed: ['trusted@example.com'], spamLevel: 'medium' }))
  const { mailbox: next } = applyIncomingFilters(mailbox)
  expect(next.find(mail => mail.id === 'b')?.folder).toBe('Spam')
  expect(next.find(mail => mail.id === 'c')?.folder).toBe('Inbox')
})

it('applies enabled rules to matching mail and leaves seed folders alone', () => {
  localStorage.setItem('harbor-mail:rules', JSON.stringify([
    { id: 'r1', name: 'News', enabled: true, conditions: [{ field: 'from', value: 'news@harbor.co' }], actions: [{ kind: 'label', value: 'Internal' }] },
  ]))
  const { mailbox: next } = applyIncomingFilters(mailbox)
  expect(next.find(mail => mail.id === 'a')?.label).toBe('Internal')
  expect(next.find(mail => mail.id === 'd')?.folder).toBe('Sent')
})

it('applies move, archive, mark-read and mark-starred actions together', () => {
  localStorage.setItem('harbor-mail:rules', JSON.stringify([
    { id: 'r2', name: 'VIP', enabled: true, conditions: [{ field: 'subject', value: 'hello' }], actions: [{ kind: 'move', value: 'Starred' }, { kind: 'mark-read' }, { kind: 'mark-starred' }] },
  ]))
  const { mailbox: next } = applyIncomingFilters(mailbox)
  const mailB = next.find(mail => mail.id === 'b')
  expect(mailB?.folder).toBe('Starred')
  expect(mailB?.unread).toBe(false)
  expect(mailB?.starred).toBe(true)
})

it('collects forwarding destinations from rules with a forward action', () => {
  localStorage.setItem('harbor-mail:rules', JSON.stringify([
    { id: 'r3', name: 'Fwd', enabled: true, conditions: [{ field: 'from', value: 'news@harbor.co' }], actions: [{ kind: 'forward', value: 'team@harbor.co' }] },
  ]))
  const { report } = applyIncomingFilters(mailbox)
  expect(report.forwarded).toEqual([{ address: 'team@harbor.co', count: 1 }])
})

it('ignores disabled rules', () => {
  localStorage.setItem('harbor-mail:rules', JSON.stringify([
    { id: 'r4', name: 'Off', enabled: false, conditions: [{ field: 'from', value: 'news@harbor.co' }], actions: [{ kind: 'label', value: 'Internal' }] },
  ]))
  const { mailbox: next } = applyIncomingFilters(mailbox)
  expect(next.find(mail => mail.id === 'a')?.label).toBe('Finance')
})

it('is idempotent across repeated passes', () => {
  localStorage.setItem('harbor-mail:spam', JSON.stringify({ blocked: ['bot@example.com'], allowed: [], spamLevel: 'medium' }))
  localStorage.setItem('harbor-mail:rules', JSON.stringify([
    { id: 'r1', name: 'News', enabled: true, conditions: [{ field: 'from', value: 'news@harbor.co' }], actions: [{ kind: 'label', value: 'Internal' }] },
  ]))
  const first = applyIncomingFilters(mailbox).mailbox
  const second = applyIncomingFilters(first).mailbox
  expect(second).toEqual(first)
})

it('matches the size condition using estimated message size', () => {
  const big = { ...mailbox[0], id: 'e', attachment: true, attachmentName: 'final-reviewed-and-signed-annual-contract-update-v2.pdf' }
  const small = { ...mailbox[1], id: 'f', preview: 'ok' }
  localStorage.setItem('harbor-mail:rules', JSON.stringify([
    { id: 'r5', name: 'Large mail', enabled: true, conditions: [{ field: 'size', op: 'larger', size: 1 }], actions: [{ kind: 'label', value: 'Bulk' }] },
    { id: 'r6', name: 'Tiny mail', enabled: true, conditions: [{ field: 'size', op: 'smaller', size: 2 }], actions: [{ kind: 'archive' }] },
  ]))
  const { mailbox: next } = applyIncomingFilters([big, small])
  expect(next.find(mail => mail.id === 'e')?.label).toBe('Bulk')
  expect(next.find(mail => mail.id === 'e')?.folder).toBe('Inbox')
  expect(next.find(mail => mail.id === 'f')?.folder).toBe('Archive')
})

it('matches the date condition against the message time', () => {
  const today = new Date().toISOString().slice(0, 10)
  const fresh = { ...mailbox[1], id: 'g', time: '9:42 AM' }
  const old = { ...mailbox[2], id: 'h', time: 'Yesterday' }
  localStorage.setItem('harbor-mail:rules', JSON.stringify([
    { id: 'r7', name: 'Recent', enabled: true, conditions: [{ field: 'date', op: 'on', value: today }], actions: [{ kind: 'mark-starred' }] },
    { id: 'r8', name: 'Older', enabled: true, conditions: [{ field: 'date', op: 'before', value: today }], actions: [{ kind: 'label', value: 'Old' }] },
  ]))
  const { mailbox: next } = applyIncomingFilters([fresh, old])
  expect(next.find(mail => mail.id === 'g')?.starred).toBe(true)
  expect(next.find(mail => mail.id === 'h')?.label).toBe('Old')
})

it('discards matching mail to Trash for a single incoming message', () => {
  const single = { ...mailbox[1], id: 'i', email: 'spammy@example.com', subject: 'You won a prize now' }
  localStorage.setItem('harbor-mail:rules', JSON.stringify([
    { id: 'r9', name: 'Junk', enabled: true, conditions: [{ field: 'subject', value: 'prize' }], actions: [{ kind: 'discard' }] },
  ]))
  const { mailbox: next } = applyIncomingFilters([single])
  expect(next.find(mail => mail.id === 'i')?.folder).toBe('Trash')
})

it('reports forwarding for a single incoming message but keeps it in the mailbox', () => {
  const single = { ...mailbox[0], id: 'j', email: 'news@harbor.co' }
  localStorage.setItem('harbor-mail:rules', JSON.stringify([
    { id: 'r10', name: 'Shine a light', enabled: true, conditions: [{ field: 'from', value: 'news@harbor.co' }], actions: [{ kind: 'forward', value: 'team@harbor.co' }] },
  ]))
  const { mailbox: next, report } = applyIncomingFilters([single])
  expect(report.forwarded).toEqual([{ address: 'team@harbor.co', count: 1 }])
  expect(next.find(mail => mail.id === 'j')?.folder).toBe('Inbox')
})