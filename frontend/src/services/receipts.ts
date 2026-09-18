import type { Mail } from '../types'
import { apiFetch } from '../lib/api'
import { isRemoteMail } from './remote-mail'

export type ReadReceipt = {
  id: string
  mailId: string
  sender: string
  email: string
  subject: string
  at: string
}

type ReceiptRow = {
  id: string
  mailId: string
  sender: string
  email: string
  subject: string
  at: string
}

const receiptsKey = 'harbor-mail:read-receipts'

const readReceipts = (): ReadReceipt[] => {
  try {
    const raw = localStorage.getItem(receiptsKey)
    return raw ? (JSON.parse(raw) as ReadReceipt[]) : []
  } catch {
    return []
  }
}

const writeReceipts = (items: ReadReceipt[]) => {
  try {
    localStorage.setItem(receiptsKey, JSON.stringify(items))
  } catch {
    /* quota exceeded — the in-memory list still renders this session */
  }
}

export const receiptsApi = {
  list(): ReadReceipt[] {
    return readReceipts()
  },
  save(items: ReadReceipt[]) {
    writeReceipts(items)
  },
  /** API-first refresh; falls back to the local cache when offline or in demo mode. */
  async refresh(): Promise<ReadReceipt[]> {
    if (!isRemoteMail()) return this.list()
    try {
      const result = await apiFetch<{ receipts: ReceiptRow[] }>('/api/receipts')
      const next = (result.receipts ?? []).map((row) => ({ ...row }))
      writeReceipts(next)
      return next
    } catch {
      return this.list()
    }
  },
  has(mailId: string): boolean {
    return this.list().some((receipt) => receipt.mailId === mailId)
  },
  async record(mail: Mail): Promise<ReadReceipt> {
    const receipt: ReadReceipt = {
      id: `receipt-${Date.now()}`,
      mailId: mail.id,
      sender: mail.sender,
      email: mail.email,
      subject: mail.subject,
      at: new Date().toISOString(),
    }
    if (isRemoteMail()) {
      try {
        const row = await apiFetch<ReceiptRow>('/api/receipts', {
          method: 'POST',
          body: JSON.stringify({
            mailId: mail.id,
            sender: mail.sender,
            email: mail.email,
            subject: mail.subject,
          }),
        })
        const next = [row, ...this.list().filter((item) => item.mailId !== mail.id)]
        writeReceipts(next)
        return row
      } catch {
        // Offline: keep the local receipt so the UI still reflects it.
      }
    }
    writeReceipts([receipt, ...this.list()])
    return receipt
  },
}

export type ReceiptRequest = { id: string; mailId: string; recipient: string; at: string }

type RequestRow = { id: string; mailId: string; recipient: string; at: string }

const requestsKey = 'harbor-mail:receipt-requests'

const readRequests = (): ReceiptRequest[] => {
  try {
    const raw = localStorage.getItem(requestsKey)
    return raw ? (JSON.parse(raw) as ReceiptRequest[]) : []
  } catch {
    return []
  }
}

const writeRequests = (items: ReceiptRequest[]) => {
  try {
    localStorage.setItem(requestsKey, JSON.stringify(items))
  } catch {
    /* quota exceeded — the in-memory list still renders this session */
  }
}

export const receiptRequestsApi = {
  list(): ReceiptRequest[] {
    return readRequests()
  },
  save(items: ReceiptRequest[]) {
    writeRequests(items)
  },
  /** API-first refresh; falls back to the local cache when offline or in demo mode. */
  async refresh(): Promise<ReceiptRequest[]> {
    if (!isRemoteMail()) return this.list()
    try {
      const result = await apiFetch<{ requests: RequestRow[] }>('/api/receipt-requests')
      const next = (result.requests ?? []).map((row) => ({ ...row }))
      writeRequests(next)
      return next
    } catch {
      return this.list()
    }
  },
  wasRequested(mailId: string): boolean {
    return this.list().some((request) => request.mailId === mailId)
  },
  async request(mailId: string, recipient: string): Promise<ReceiptRequest> {
    const record: ReceiptRequest = {
      id: `request-${Date.now()}`,
      mailId,
      recipient,
      at: new Date().toISOString(),
    }
    if (isRemoteMail()) {
      try {
        const row = await apiFetch<RequestRow>('/api/receipt-requests', {
          method: 'POST',
          body: JSON.stringify({ mailId, recipient }),
        })
        const next = [row, ...this.list().filter((item) => item.mailId !== mailId)]
        writeRequests(next)
        return row
      } catch {
        // Offline: keep the local request so the UI still reflects it.
      }
    }
    writeRequests([record, ...this.list()])
    return record
  },
}
