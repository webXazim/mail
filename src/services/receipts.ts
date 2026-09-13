import type { Mail } from '../types'

export type ReadReceipt = {
  id: string
  mailId: string
  sender: string
  email: string
  subject: string
  at: string
}

const receiptsKey = 'harbor-mail:read-receipts'

export const receiptsApi = {
  list(): ReadReceipt[] {
    try {
      const raw = localStorage.getItem(receiptsKey)
      return raw ? (JSON.parse(raw) as ReadReceipt[]) : []
    } catch {
      return []
    }
  },
  save(items: ReadReceipt[]) {
    localStorage.setItem(receiptsKey, JSON.stringify(items))
  },
  has(mailId: string): boolean {
    return this.list().some((receipt) => receipt.mailId === mailId)
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

export type ReceiptRequest = { id: string; mailId: string; recipient: string; at: string }

const requestsKey = 'harbor-mail:receipt-requests'

export const receiptRequestsApi = {
  list(): ReceiptRequest[] {
    try {
      const raw = localStorage.getItem(requestsKey)
      return raw ? (JSON.parse(raw) as ReceiptRequest[]) : []
    } catch {
      return []
    }
  },
  save(items: ReceiptRequest[]) {
    localStorage.setItem(requestsKey, JSON.stringify(items))
  },
  wasRequested(mailId: string): boolean {
    return this.list().some((request) => request.mailId === mailId)
  },
  request(mailId: string, recipient: string): ReceiptRequest {
    const record: ReceiptRequest = {
      id: `request-${Date.now()}`,
      mailId,
      recipient,
      at: new Date().toISOString(),
    }
    this.save([record, ...this.list()])
    return record
  },
}
