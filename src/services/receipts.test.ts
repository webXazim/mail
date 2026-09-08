import { beforeEach, describe, expect, it } from 'vitest'
import { receiptsApi } from './receipts'
import type { Mail } from '../types'

const mail: Mail = { id: 'm1', initials: 'NS', sender: 'Nora Salas', email: 'nora@harbor.co', subject: 'Launch', preview: '', time: '', label: 'Inbox', color: 'blue', unread: false }

beforeEach(() => localStorage.clear())

describe('receiptsApi', () => {
  it('records a receipt once per message', () => {
    expect(receiptsApi.has(mail.id)).toBe(false)
    receiptsApi.record(mail)
    expect(receiptsApi.has(mail.id)).toBe(true)
    expect(receiptsApi.list()).toHaveLength(1)
    receiptsApi.record(mail)
    expect(receiptsApi.list()).toHaveLength(2)
  })

  it('stores the receipt metadata', () => {
    receiptsApi.record(mail)
    const [receipt] = receiptsApi.list()
    expect(receipt.mailId).toBe('m1')
    expect(receipt.sender).toBe('Nora Salas')
    expect(receipt.subject).toBe('Launch')
  })

  it('survives a reload', () => {
    receiptsApi.record(mail)
    expect(receiptsApi.has('m1')).toBe(true)
  })
})