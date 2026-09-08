import type { Mail } from '../types'

export type ReadReceipt = { id: string; mailId: string; sender: string; email: string; subject: string; at: string }

const receiptsKey = 'harbor-mail:read-receipts'

export const receiptsApi = {
  list(): ReadReceipt[] {
    try {
      const raw = localStorage.getItem(receiptsKey)
      return raw ? JSON.parse(raw) as ReadReceipt[] : []
    } catch {
      return []
    }
  },
  save(items: ReadReceipt[]) {
    localStorage.setItem(receiptsKey, JSON.stringify(items))
  },
  has(mailId: string): boolean {
    return this.list().some(receipt => receipt.mailId === mailId)
  },
  record(mail: Mail): ReadReceipt {
    const receipt: ReadReceipt = {
      id: `receipt-${Date.now()}`,
      mailId: mail.id,
      sender: mail.sender,
      email: mail.email,
      subject: mail.subject,
      at: new Date().toISOString(),
    }
    this.save([receipt, ...this.list()])
    return receipt
  },
}