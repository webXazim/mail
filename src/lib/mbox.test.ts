import { describe, expect, it } from 'vitest'
import type { Mail } from '../types'
import { exportToMbox, importFromMbox } from './mbox'

const mail: Mail = { id: 'm1', initials: 'PK', sender: 'Priya Khan', email: 'priya@harbor.co', subject: 'Design review notes', preview: 'I have updated the project details…', time: '10:24 AM', label: 'Clients', color: 'coral', unread: true, attachment: true, attachmentName: 'meridian-contract-signed.pdf' }

describe('exportToMbox', () => {
  it('writes a From line and Harbor metadata headers', () => {
    const exported = exportToMbox([mail])
    expect(exported).toContain(`From alex@harbor.co ${mail.time}`)
    expect(exported).toContain(`From: ${mail.sender} <${mail.email}>`)
    expect(exported).toContain(`Subject: ${mail.subject}`)
    expect(exported).toContain('X-Harbor-Id: m1')
    expect(exported).toContain('X-Harbor-Folder: Inbox')
    expect(exported).toContain('X-Harbor-Unread: yes')
    expect(exported).toContain('X-Harbor-Starred: no')
    expect(exported).toContain('X-Harbor-Attachment: yes')
    expect(exported).toContain('X-Harbor-Attachment-Name: meridian-contract-signed.pdf')
    expect(exported).toContain('From: Priya Khan')
  })

  it('escapes body lines that begin with From ', () => {
    const tricky = { ...mail, preview: 'From line one\nNormal line\nFrom another' }
    const exported = exportToMbox([tricky])
    expect(exported).toContain('>From line one')
    expect(exported).toContain('>From another')
    expect(exported).toContain('\nNormal line\n')
  })

  it('joins multiple messages as separate blocks', () => {
    const second = { ...mail, id: 'm2', subject: 'Second subject' }
    const exported = exportToMbox([mail, second])
    expect(exported).toContain('X-Harbor-Id: m1')
    expect(exported).toContain('X-Harbor-Id: m2')
    expect(exported.split(/\n(?=From alex)/).length).toBe(2)
  })
})

describe('importFromMbox', () => {
  it('round-trips an exported mailbox', () => {
    const mails = [
      mail,
      { ...mail, id: 'm2', sender: 'Dana Weiss', email: 'dana@elsewhere.io', subject: 'Catch up', preview: 'Free Friday morning?', time: 'Yesterday', label: 'Social', color: 'teal', unread: false, starred: true, folder: 'Archive', to: ['alex@harbor.co'] },
    ]
    const imported = importFromMbox(exportToMbox(mails))
    expect(imported).toHaveLength(2)
    expect(imported[0].id).toBe('imported-m1')
    expect(imported[0].subject).toBe('Design review notes')
    expect(imported[0].preview).toBe('I have updated the project details…')
    expect(imported[0].folder).toBe('Inbox')
    expect(imported[0].unread).toBe(true)
    expect(imported[0].attachmentName).toBe('meridian-contract-signed.pdf')
    expect(imported[1].starred).toBe(true)
    expect(imported[1].folder).toBe('Archive')
    expect(imported[1].to).toEqual(['alex@harbor.co'])
  })

  it('unescapes >From body lines', () => {
    const tricky = { ...mail, preview: '>From line one\nNormal line\nFrom another' }
    const imported = importFromMbox(exportToMbox([tricky]))
    expect(imported).toHaveLength(1)
    expect(imported[0].preview).toBe('From line one\nNormal line\nFrom another')
  })

  it('skips blocks without a Harbor id and keeps valid ones', () => {
    const foreign = 'From someone@elsewhere.net\nSubject: Raw email\n\nPlain body'
    const good = exportToMbox([mail])
    const imported = importFromMbox(`${foreign}\n\n${good}`)
    expect(imported).toHaveLength(1)
    expect(imported[0].id).toBe('imported-m1')
  })
})