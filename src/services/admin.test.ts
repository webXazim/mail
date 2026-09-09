import { beforeEach, describe, expect, it } from 'vitest'
import { adminApi, seedQuarantine } from './admin'
import { primaryAccountId } from './accounts'
import { mailboxApi } from './mailbox'

beforeEach(() => localStorage.clear())

describe('adminApi', () => {
  it('seeds the default security and quarantine state', () => {
    const security = adminApi.getSecurity()
    expect(security.spamThreshold).toBe(5)
    expect(security.dmarcPolicy).toBe('quarantine')
    expect(adminApi.listQuarantine()).toHaveLength(seedQuarantine.length)
  })

  it('changes mailbox status and quota while protecting the primary mailbox', () => {
    const statusNext = adminApi.setMailboxStatus('mb-nora', 'quarantine')
    expect(statusNext.find(mailbox => mailbox.id === 'mb-nora')?.status).toBe('quarantine')
    const quotaNext = adminApi.setMailboxQuota('mb-nora', 25)
    expect(quotaNext.find(mailbox => mailbox.id === 'mb-nora')?.quotaGB).toBe(25)
    const removed = adminApi.removeMailbox('mb-alex')
    expect(removed.find(mailbox => mailbox.id === 'mb-alex')).toBeTruthy()
  })

  it('resets a mailbox password and makes it retrievable', () => {
    const temp = adminApi.resetPassword('mb-nora')
    expect(temp.startsWith('hap-')).toBe(true)
    expect(adminApi.tempPassword('nora@harbor.co')).toBe(temp)
  })

  it('adds and removes blocked senders', () => {
    const withSender = adminApi.addBlockedSender('nope@bad.example')
    expect(withSender.blockedSenders).toContain('nope@bad.example')
    const without = adminApi.removeBlockedSender('nope@bad.example')
    expect(without.blockedSenders).not.toContain('nope@bad.example')
  })

  it('releases a quarantined message into the primary inbox', async () => {
    const target = adminApi.listQuarantine()[0]
    const next = await adminApi.releaseQuarantine(target.id)
    expect(next.find(message => message.id === target.id)).toBeUndefined()
    const inbox = await mailboxApi.listFor(primaryAccountId)
    expect(inbox[0].subject).toBe(target.subject)
    expect(inbox[0].accountId).toBe(primaryAccountId)
  })

  it('prepends audit entries to the seeded log', () => {
    adminApi.logAudit('Test action', 'some detail')
    const audit = adminApi.listAudit()
    expect(audit[0].action).toBe('Test action')
    expect(audit[0].detail).toBe('some detail')
    expect(audit).toHaveLength(4)
  })
})