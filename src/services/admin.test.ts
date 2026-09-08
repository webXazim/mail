import { beforeEach, describe, expect, it } from 'vitest'
import { adminApi, seedAliases, seedForwarders, seedMailboxes } from './admin'

beforeEach(() => localStorage.clear())

describe('adminApi mailboxes', () => {
  it('starts with the seeded domain mailboxes', () => {
    const mailboxes = adminApi.listMailboxes()
    expect(mailboxes).toHaveLength(seedMailboxes.length)
    expect(mailboxes.some(mailbox => mailbox.email === 'alex@harbor.co')).toBe(true)
  })

  it('adds a mailbox on the domain when only a local part is given', () => {
    adminApi.addMailbox({ email: 'billing', displayName: 'Billing Team' })
    const mailbox = adminApi.listMailboxes().find(item => item.email === 'billing@harbor.co')
    expect(mailbox?.displayName).toBe('Billing Team')
  })

  it('keeps the address when a full email is provided', () => {
    adminApi.addMailbox({ email: 'team@acme.test', displayName: 'Acme' })
    expect(adminApi.listMailboxes().some(mailbox => mailbox.email === 'team@acme.test')).toBe(true)
  })

  it('rejects duplicates and invalid addresses', () => {
    const before = adminApi.listMailboxes().length
    expect(adminApi.addMailbox({ email: 'alex@harbor.co' }).length).toBe(before)
    expect(adminApi.addMailbox({ email: 'not an email' }).length).toBe(before)
  })

  it('protects the primary mailbox from removal', () => {
    const target = adminApi.listMailboxes().find(mailbox => mailbox.email === 'alex@harbor.co')
    if (!target) return
    expect(adminApi.removeMailbox(target.id)).toHaveLength(seedMailboxes.length)
  })

  it('removes non-primary mailboxes', () => {
    const target = adminApi.listMailboxes().find(mailbox => mailbox.email === 'dev@harbor.co')
    if (!target) return
    expect(adminApi.removeMailbox(target.id).length).toBe(seedMailboxes.length - 1)
  })
})

describe('adminApi aliases', () => {
  it('starts with the seeded aliases', () => {
    expect(adminApi.listAliases()).toHaveLength(seedAliases.length)
    expect(adminApi.listAliases().some(alias => alias.address === 'support@harbor.co')).toBe(true)
  })

  it('normalizes the local part onto the domain', () => {
    adminApi.addAlias('press', 'alex@harbor.co', 'harbor.co')
    expect(adminApi.listAliases().some(alias => alias.address === 'press@harbor.co')).toBe(true)
  })

  it('rejects duplicate aliases', () => {
    const before = adminApi.listAliases().length
    expect(adminApi.addAlias('support', 'alex@harbor.co', 'harbor.co').length).toBe(before)
  })

  it('removes aliases', () => {
    adminApi.removeAlias(seedAliases[0].id)
    expect(adminApi.listAliases().some(alias => alias.id === seedAliases[0].id)).toBe(false)
  })
})

describe('adminApi forwarders', () => {
  it('starts with the seeded forwarders', () => {
    expect(adminApi.listForwarders()).toHaveLength(seedForwarders.length)
  })

  it('adds and toggles a forwarder', () => {
    const previous = adminApi.listForwarders().length
    adminApi.addForwarder('dev@harbor.co', 'external@example.net')
    let forwarders = adminApi.listForwarders()
    expect(forwarders).toHaveLength(previous + 1)
    const created = forwarders.find(forwarder => forwarder.from === 'dev@harbor.co')
    if (!created) return
    forwarders = adminApi.toggleForwarder(created.id)
    expect(forwarders.find(forwarder => forwarder.id === created.id)?.enabled).toBe(false)
  })

  it('rejects forwarders with an invalid target', () => {
    const before = adminApi.listForwarders().length
    expect(adminApi.addForwarder('dev@harbor.co', 'not-an-email').length).toBe(before)
  })

  it('removes forwarders', () => {
    adminApi.removeForwarder(seedForwarders[0].id)
    expect(adminApi.listForwarders()).toHaveLength(0)
  })
})

describe('adminApi domain', () => {
  it('starts with no verified records and turns them all on with verifyAll', () => {
    expect(adminApi.getDns()).toEqual({ mx: false, spf: false, dkim: false, dmarc: false })
    expect(adminApi.verifyAll()).toEqual({ mx: true, spf: true, dkim: true, dmarc: true })
    expect(adminApi.getDns().mx).toBe(true)
  })

  it('persists the catch-all selection', () => {
    adminApi.saveDomainSettings({ domain: 'harbor.co', catchAllEnabled: true, catchAll: 'dev@harbor.co' })
    expect(adminApi.getDomainSettings().catchAll).toBe('dev@harbor.co')
  })
})