import { beforeEach, describe, expect, it } from 'vitest'
import { accountsApi, primaryAccount, primaryAccountId, seedAccountMailbox } from './accounts'

beforeEach(() => localStorage.clear())

describe('accountsApi', () => {
  it('starts with only the primary account', () => {
    expect(accountsApi.list()).toEqual([primaryAccount])
  })

  it('adds an account, seeds its mailbox, and returns both', () => {
    const { account, mailbox } = accountsApi.add({ name: 'Amira Khalil', email: 'amira@northbeam.dev', password: 'secret123' })
    expect(account.email).toBe('amira@northbeam.dev')
    expect(account.id).not.toBe(primaryAccountId)
    expect(accountsApi.list()).toHaveLength(2)
    expect(localStorage.getItem(`harbor-mail:mailbox:${account.id}`)).not.toBeNull()
    expect(mailbox.length).toBeGreaterThan(0)
    expect(mailbox[0].accountId).toBe(account.id)
  })

  it('rejects an invalid email, a short password, or a duplicate email', () => {
    expect(() => accountsApi.add({ name: 'X', email: 'not-an-email', password: 'secret123' })).toThrow()
    expect(() => accountsApi.add({ name: 'X', email: 'ok@northbeam.dev', password: '123' })).toThrow()
    accountsApi.add({ name: 'Amira Khalil', email: 'amira@northbeam.dev', password: 'secret123' })
    expect(() => accountsApi.add({ name: 'Amira', email: 'amira@northbeam.dev', password: 'secret123' })).toThrow()
  })

  it('protects the primary account from removal and clears another account data', () => {
    const { account } = accountsApi.add({ name: 'Amira Khalil', email: 'amira@northbeam.dev', password: 'secret123' })
    accountsApi.remove(primaryAccountId)
    expect(accountsApi.list()).toHaveLength(2)
    accountsApi.remove(account.id)
    expect(accountsApi.list()).toHaveLength(1)
    expect(localStorage.getItem(`harbor-mail:mailbox:${account.id}`)).toBeNull()
  })

  it('seeds mail addressed to the linked account', () => {
    const seed = seedAccountMailbox('account-x', 'amira@northbeam.dev')
    expect(seed.every(mail => mail.to?.includes('amira@northbeam.dev'))).toBe(true)
    expect(seed.every(mail => mail.accountId === 'account-x')).toBe(true)
  })
})