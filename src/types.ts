export type Mailbox = 'Inbox' | 'Unread' | 'Starred' | 'Snoozed' | 'Sent' | 'Scheduled' | 'Drafts' | 'All Mail' | 'Archive' | 'Spam' | 'Trash'
export type Mail = { id:string; initials:string; sender:string; email:string; subject:string; preview:string; time:string; label:string; color:string; unread:boolean; attachment?:boolean; attachmentName?:string; starred?:boolean; folder?: string; to?: string[]; cc?: string[]; snoozedUntil?: string }
export type Draft = { to: string; cc: string; bcc: string; subject: string; body: string; attachments: string[]; scheduledAt: string }
