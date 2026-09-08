import { describe, expect, it } from 'vitest'
import { messages } from '../data'
import type { Draft, Mail } from '../types'
import { buildForwardDraft, buildReplyAllDraft, buildReplyDraft, buildSentMail, buildThread, filterMails, getFolderCounts, matchesSearch, snoozeAt } from './mail'

const mail: Mail = { id: 'm1', initials: 'PK', sender: 'Priya Khan', email: 'priya@harbor.co', subject: 'Design review notes', preview: 'I have updated the project details…', time: '10:24 AM', label: 'Clients', color: 'coral', unread: true, attachment: true, attachmentName: 'meridian-contract-signed.pdf' }

describe('buildReplyDraft', () => {
  it('prefills the recipient and prefixes Re:', () => {
    const draft = buildReplyDraft(mail)
    expect(draft.to).toBe('priya@harbor.co')
    expect(draft.subject).toBe('Re: Design review notes')
  })

  it('does not double-prefix Re:', () => {
    const draft = buildReplyDraft({ ...mail, subject: 'Re: Design review notes' })
    expect(draft.subject).toBe('Re: Design review notes')
  })
})

describe('buildForwardDraft', () => {
  it('prefixes Fwd: and leaves recipients empty', () => {
    const draft = buildForwardDraft(mail)
    expect(draft.to).toBe('')
    expect(draft.subject).toBe('Fwd: Design review notes')
  })

  it('quotes the original message and carries the attachment name', () => {
    const draft = buildForwardDraft(mail)
    expect(draft.body).toContain('---------- Forwarded message ----------')
    expect(draft.body).toContain('From: Priya Khan <priya@harbor.co>')
    expect(draft.body).toContain(mail.preview)
    expect(draft.attachments).toEqual(['meridian-contract-signed.pdf'])
  })
})

describe('buildSentMail', () => {
  const draft: Draft = { to: 'priya@harbor.co', cc: '', bcc: '', subject: 'Hello', body: 'a'.repeat(200), attachments: [], scheduledAt: '' }

  it('builds a Sent mail from the draft with a truncated preview', () => {
    const sent = buildSentMail(draft)
    expect(sent.folder).toBe('Sent')
    expect(sent.time).toBe('Just now')
    expect(sent.preview).toHaveLength(120)
    expect(sent.attachment).toBe(false)
  })

  it('uses the provided time for scheduled sends', () => {
    expect(buildSentMail(draft, '3:30 PM').time).toBe('3:30 PM')
  })

  it('handles an empty subject and body', () => {
    const sent = buildSentMail({ ...draft, subject: '', body: '' })
    expect(sent.subject).toBe('(no subject)')
    expect(sent.preview).toBe('(no body)')
  })

  it('uses the selected identity for the From line', () => {
    const sent = buildSentMail({ ...draft, from: { name: 'Marketing', email: 'marketing@harbor.co' } })
    expect(sent.sender).toBe('Marketing')
    expect(sent.email).toBe('marketing@harbor.co')
    expect(sent.initials).toBe('M')
  })

  it('falls back to the profile defaults without an identity', () => {
    const sent = buildSentMail(draft)
    expect(sent.sender).toBe('Alex Morgan')
    expect(sent.email).toBe('alex@harbor.co')
    expect(sent.initials).toBe('AM')
  })
})

describe('getFolderCounts', () => {
  it('tallies folders consistently with the list page filters', () => {
    const counts = getFolderCounts(messages)
    expect(counts.Inbox).toBe(3)
    expect(counts.Unread).toBe(3)
    expect(counts.Starred).toBe(2)
    expect(counts.Sent).toBe(1)
    expect(counts.Archive).toBe(1)
    expect(counts.Spam).toBe(1)
    expect(counts.Drafts).toBe(0)
    expect(counts.Trash).toBe(0)
    expect(counts['All Mail']).toBe(messages.length)
  })

  it('excludes trashed messages from smart folders', () => {
    const trashed = messages.map(mail => ({ ...mail }))
    trashed.push({ ...messages[0], id: 't1', folder: 'Trash', unread: true, starred: true })
    const counts = getFolderCounts(trashed)
    expect(counts.Unread).toBe(3)
    expect(counts.Starred).toBe(2)
    expect(counts['All Mail']).toBe(messages.length)
    expect(counts.Trash).toBe(1)
  })
})

describe('buildThread', () => {
  it('starts with the source message and formats the reply attribution', () => {
    const thread = buildThread(mail)
    expect(thread).toHaveLength(3)
    expect(thread[0].sender).toBe(mail.sender)
    expect(thread[0].copy).toContain(mail.preview)
    expect(thread[1].copy).toContain(`On ${mail.time}, ${mail.sender} <${mail.email}> wrote:`)
  })
})

describe('buildReplyAllDraft', () => {
  it('includes the sender and other recipients but excludes yourself', () => {
    const message: Mail = { ...mail, to: ['caro@harbor.co', 'alex@harbor.co'], cc: ['ana@harbor.co', 'alex@harbor.co'] }
    const draft = buildReplyAllDraft(message)
    expect(draft.to).toBe('priya@harbor.co, caro@harbor.co')
    expect(draft.cc).toBe('ana@harbor.co')
    expect(draft.subject).toBe('Re: Design review notes')
  })
})

describe('snoozeAt', () => {
  const now = new Date('2026-09-07T10:00:00')

  it('later today targets 5pm today', () => {
    const res = new Date(snoozeAt('later-today', now))
    expect(res.getDate()).toBe(now.getDate())
    expect(res.getHours()).toBe(17)
  })

  it('later today after 5pm rolls to tomorrow 8am', () => {
    const res = new Date(snoozeAt('later-today', new Date('2026-09-07T18:00:00')))
    expect(res.getDate()).toBe(8)
    expect(res.getHours()).toBe(8)
  })

  it('tomorrow targets 8am the next day', () => {
    const res = new Date(snoozeAt('tomorrow', now))
    expect(res.getDate()).toBe(8)
    expect(res.getHours()).toBe(8)
  })

  it('this weekend targets Saturday 8am', () => {
    const res = new Date(snoozeAt('this-weekend', now))
    expect(res.getDay()).toBe(6)
    expect(res.getHours()).toBe(8)
  })

  it('next week targets Monday 8am', () => {
    const res = new Date(snoozeAt('next-week', now))
    expect(res.getDay()).toBe(1)
    expect(res.getHours()).toBe(8)
  })
})

describe('filterMails', () => {
  it('mirrors the smart folder filters from the list page', () => {
    expect(filterMails(messages, 'Inbox')).toHaveLength(3)
    expect(filterMails(messages, 'Unread')).toHaveLength(3)
    expect(filterMails(messages, 'Starred')).toHaveLength(2)
    expect(filterMails(messages, 'Sent')).toHaveLength(1)
    expect(filterMails(messages, 'All Mail')).toHaveLength(6)
  })

  it('applies inbox categories and search queries', () => {
    expect(filterMails(messages, 'Inbox', '', 'Primary')).toHaveLength(3)
    expect(filterMails(messages, 'Inbox', '', 'Updates')).toHaveLength(1)
    expect(filterMails(messages, 'Inbox', '', 'Promotions')).toHaveLength(0)
    expect(filterMails(messages, 'Inbox', 'is:unread')).toHaveLength(3)
    expect(filterMails(messages, 'Inbox', 'from:priya')).toHaveLength(1)
  })
})

describe('search operators', () => {
  const box: Mail[] = [
    { id: 'a', initials: 'NL', sender: 'Nora Li', email: 'nora@northstar.studio', subject: 'Q3 launch plan', preview: 'milestones for Thursday', time: '9:42 AM', label: 'Clients', color: 'purple', unread: true, starred: true, folder: 'Inbox', to: ['alex@harbor.co'] },
    { id: 'b', initials: 'JM', sender: 'Jonas Meier', email: 'jonas@meridian.co', subject: 'Meridian contract', preview: 'signed copy attached', time: '8:18 AM', label: 'Attachment', color: 'orange', unread: false, attachment: true, folder: 'Inbox', to: ['team@harbor.co', 'alex@harbor.co'] },
    { id: 'c', initials: 'RB', sender: 'Riley Brooks', email: 'riley@papertrail.com', subject: 'Invoice #1048', preview: 'payment due', time: 'Aug 29', label: 'Finance', color: 'coral', unread: true, folder: 'Snoozed', to: ['alex@harbor.co'] },
    { id: 'd', initials: 'X', sender: 'Old Client', email: 'old@agency.example', subject: 'Old files', preview: 'archive me', time: 'Aug 1', label: 'Clients', color: 'blue', unread: false, folder: 'Trash', to: ['alex@harbor.co'] },
  ]

  it('matches plain text across sender, subject, preview, labels and recipients', () => {
    expect(matchesSearch(box[0], 'launch')).toBe(true)
    expect(matchesSearch(box[0], 'milestones')).toBe(true)
    expect(matchesSearch(box[0], 'Clients')).toBe(true)
  })

  it('filters by to: and subject: operators', () => {
    expect(matchesSearch(box[1], 'to:team@harbor.co')).toBe(true)
    expect(matchesSearch(box[1], 'to:meridian')).toBe(false)
    expect(matchesSearch(box[2], 'subject:invoice')).toBe(true)
    expect(matchesSearch(box[2], 'subject:payment')).toBe(false)
  })

  it('differentiates is:read from is:unread', () => {
    expect(matchesSearch(box[0], 'is:unread')).toBe(true)
    expect(matchesSearch(box[1], 'is:read')).toBe(true)
    expect(matchesSearch(box[0], 'is:read')).toBe(false)
  })

  it('supports is:snoozed, is:sent and is:draft', () => {
    expect(matchesSearch(box[2], 'is:snoozed')).toBe(true)
    expect(matchesSearch(box[0], 'is:snoozed')).toBe(false)
    expect(matchesSearch({ ...box[0], folder: 'Sent' }, 'is:sent')).toBe(true)
    expect(matchesSearch({ ...box[0], folder: 'Drafts' }, 'is:draft')).toBe(true)
  })

  it('handles negated operators with a leading dash', () => {
    expect(matchesSearch(box[2], '-has:attachment')).toBe(true)
    expect(matchesSearch(box[1], '-has:attachment')).toBe(false)
    expect(matchesSearch(box[1], '-is:unread')).toBe(true)
    expect(matchesSearch(box[0], '-label:Clients')).toBe(false)
  })

  it('scopes searches across folders with in:', () => {
    expect(filterMails(box, 'Inbox', 'in:trash').map(item => item.id)).toEqual(['d'])
    expect(filterMails(box, 'Inbox', 'in:snoozed').map(item => item.id)).toEqual(['c'])
    expect(filterMails(box, 'Inbox', 'in:any subject:invoice').map(item => item.id)).toEqual(['c'])
    expect(filterMails(box, 'Inbox', 'in:all is:unread').map(item => item.id)).toEqual(['a', 'c'])
  })
})