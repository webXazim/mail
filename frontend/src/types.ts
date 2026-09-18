export type Mailbox =
  | 'Inbox'
  | 'Unread'
  | 'Starred'
  | 'Snoozed'
  | 'Sent'
  | 'Scheduled'
  | 'Drafts'
  | 'All Mail'
  | 'Archive'
  | 'Spam'
  | 'Trash'
export type Mail = {
  id: string
  initials: string
  sender: string
  email: string
  subject: string
  preview: string
  time: string
  label: string
  color: string
  unread: boolean
  attachment?: boolean
  attachmentName?: string
  starred?: boolean
  folder?: string
  to?: string[]
  cc?: string[]
  snoozedUntil?: string
  accountId?: string
  receiptRequested?: boolean
  /** Backend conversation id (real-mail mode). */
  threadId?: string
}
export type SecurityVerdicts = {
  spf: 'pass' | 'fail' | 'softfail' | 'neutral' | 'none' | 'temperror' | 'permerror' | string
  dkim: SecurityVerdicts['spf']
  dmarc: SecurityVerdicts['spf']
  spam: boolean
  score: number
}
export type ReaderThreadItem = {
  id: string
  threadId: string
  sender: string
  email: string
  initials: string
  color: string
  copy: string
  bodyHtml: string
  time: string
  to?: string[]
  cc?: string[]
  attachments?: { name: string; blobId: string; type: string }[]
  messageId?: string
  security?: SecurityVerdicts
}
export type Draft = {
  to: string
  cc: string
  bcc: string
  subject: string
  body: string
  attachments: string[]
  scheduledAt: string
  from?: { name: string; email: string }
  receiptRequested?: boolean
}
