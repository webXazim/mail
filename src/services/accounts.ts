import type { Mail } from '../types'

export type Account = { id: string; name: string; email: string; initials: string; color: string; provider: string }

const accountsKey = 'harbor-mail:accounts'
const mailboxKeyFor = (accountId: string) => `harbor-mail:mailbox:${accountId}`

export const primaryAccountId = 'account-primary'
export const unifiedViewId = 'unified'
export const relatedColors = ['coral', 'purple', 'blue', 'green', 'orange']

const nameInitials = (name: string) =>
  name.trim().split(/\s+/).map(part => part[0]?.toUpperCase() ?? '').slice(0, 2).join('') || '?'

export const primaryAccount: Account = {
  id: primaryAccountId,
  name: 'Alex Morgan',
  email: 'alex@harbor.co',
  initials: 'AM',
  color: 'teal',
  provider: 'Harbor Mail',
}

export function seedAccountMailbox(accountId: string, email: string): Mail[] {
  return [
    { id: `${accountId}-n1`, initials: 'MC', sender: 'Maya Chen', email: 'maya@brightleaf.co', subject: 'Weekend offsite at the lighthouse', preview: 'Booked the ferry for us — two rooms are confirmed.', time: '10:04 AM', label: 'Internal', color: 'coral', unread: true, to: [email], accountId },
    { id: `${accountId}-n2`, initials: 'DP', sender: 'Dimitri Petrov', email: 'dimitri@northwind.io', subject: 'Re: Feature freeze timeline', preview: 'Locked the milestone dates and cleared the queue for the next sprint.', time: 'Yesterday', label: 'Clients', color: 'blue', unread: false, to: [email], accountId },
    { id: `${accountId}-n3`, initials: 'SM', sender: 'Sofia Marin', email: 'sofia@plantera.co', subject: 'Menu tasting + venue check', preview: 'We are confirmed for Tuesday at 6. Bring the shortlist.', time: '9:15 AM', label: 'Attachment', color: 'purple', unread: true, attachment: true, attachmentName: 'venue-checklist.pdf', to: [email], accountId },
    { id: `${accountId}-n4`, initials: 'HB', sender: 'Harbor Mail', email: 'no-reply@harbor.co', subject: 'Your March statement is ready', preview: 'View this month policy details and billing summary.', time: 'Mar 06', label: 'Finance', color: 'green', unread: false, to: [email], accountId },
  ]
}

export const accountsApi = {
  list(): Account[] {
    try {
      const parsed = JSON.parse(localStorage.getItem(accountsKey) || '[]') as Account[]
      return [primaryAccount, ...(Array.isArray(parsed) ? parsed : [])]
    } catch {
      return [primaryAccount]
    }
  },
  add(input: { name: string; email: string; password: string }): { account: Account; mailbox: Mail[] } {
    const email = input.email.trim().toLowerCase()
    if (!email.includes('@') || input.password.length < 6) throw new Error('Enter a valid email and a password of at least 6 characters')
    const existing = this.list()
    if (existing.some(account => account.email === email)) throw new Error('That account is already linked')
    const name = input.name.trim() || email.split('@')[0]
    const account: Account = {
      id: `account-${Date.now()}`,
      name,
      email,
      initials: nameInitials(name),
      color: relatedColors[(existing.length - 1) % relatedColors.length],
      provider: 'Harbor Mail',
    }
    const mailbox = seedAccountMailbox(account.id, email)
    localStorage.setItem(accountsKey, JSON.stringify([...existing.slice(1), account]))
    localStorage.setItem(mailboxKeyFor(account.id), JSON.stringify(mailbox))
    return { account, mailbox }
  },
  remove(id: string): Account[] {
    if (id === primaryAccountId) return this.list()
    const next = this.list().filter(account => account.id !== id)
    localStorage.setItem(accountsKey, JSON.stringify(next.slice(1)))
    localStorage.removeItem(mailboxKeyFor(id))
    return next
  },
}