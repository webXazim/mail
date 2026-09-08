import type { Mail } from '../../types'

export type MailActionKind = 'archive' | 'read' | 'trash'

export type MailUndo = { previous: Mail[]; ids: string[] }

export type MailState = {
  mailbox: Mail[]
  loading: boolean
  loadError: boolean
  notice: string
  undo: MailUndo | null
}

export type MailboxAction =
  | { type: 'hydrated'; mailbox: Mail[] }
  | { type: 'load-failed' }
  | { type: 'retry' }
  | { type: 'mark-read'; ids: string[] }
  | { type: 'mark-unread'; ids: string[] }
  | { type: 'mark-all-read'; ids: string[] }
  | { type: 'toggle-star'; ids: string[] }
  | { type: 'toggle-label'; ids: string[]; label: string }
  | { type: 'apply'; ids: string[]; action: MailActionKind }
  | { type: 'move-to'; ids: string[]; folder: string }
  | { type: 'empty-trash' }
  | { type: 'snooze'; ids: string[]; until: string }
  | { type: 'unsnooze' }
  | { type: 'undo' }
  | { type: 'too-late' }
  | { type: 'sent'; mail: Mail }
  | { type: 'receive'; mail: Mail }
  | { type: 'unsend-mail'; id: string }
  | { type: 'notice'; message: string }
  | { type: 'clear-notice' }

export const initialMailState: MailState = { mailbox: [], loading: true, loadError: false, notice: '', undo: null }

export const applyNotice: Record<MailActionKind, string> = {
  archive: 'Archived',
  read: 'Marked as read',
  trash: 'Moved to Trash',
}

const applyKind = (mailbox: Mail[], ids: string[], action: MailActionKind): Mail[] =>
  mailbox.map(mail => {
    if (!ids.includes(mail.id)) return mail
    if (action === 'read') return { ...mail, unread: false }
    return { ...mail, folder: action === 'trash' ? 'Trash' : 'Archive' }
  })

const atIds = (ids: string[], mail: Mail) => ids.includes(mail.id)

export function mailboxReducer(state: MailState, action: MailboxAction): MailState {
  switch (action.type) {
    case 'hydrated':
      return { ...state, mailbox: action.mailbox, loading: false, loadError: false }
    case 'load-failed':
      return { ...state, loading: false, loadError: true, notice: 'Unable to sync mailbox' }
    case 'retry':
      return { ...state, loading: true, loadError: false, notice: '' }
    case 'mark-read':
      return { ...state, mailbox: state.mailbox.map(mail => atIds(action.ids, mail) ? { ...mail, unread: false } : mail) }
    case 'mark-unread':
      if (!action.ids.length) return { ...state, notice: 'Select at least one message first' }
      return {
        ...state,
        mailbox: state.mailbox.map(mail => atIds(action.ids, mail) ? { ...mail, unread: true } : mail),
        undo: { previous: state.mailbox, ids: action.ids },
        notice: 'Marked as unread',
      }
    case 'mark-all-read':
      if (!action.ids.length) return { ...state, notice: 'Nothing to mark read' }
      return {
        ...state,
        mailbox: state.mailbox.map(mail => atIds(action.ids, mail) ? { ...mail, unread: false } : mail),
        undo: { previous: state.mailbox, ids: action.ids },
        notice: 'Marked all as read',
      }
    case 'toggle-star':
      if (!action.ids.length) return state
      return {
        ...state,
        mailbox: state.mailbox.map(mail => atIds(action.ids, mail) ? { ...mail, starred: !mail.starred } : mail),
        undo: { previous: state.mailbox, ids: action.ids },
        notice: 'Updated star',
      }
    case 'toggle-label':
      if (!action.ids.length) return { ...state, notice: 'Select at least one message first' }
      return {
        ...state,
        mailbox: state.mailbox.map(mail => atIds(action.ids, mail) ? { ...mail, label: mail.label === action.label ? '' : action.label } : mail),
        undo: { previous: state.mailbox, ids: action.ids },
        notice: 'Updated label',
      }
    case 'apply':
      if (!action.ids.length) return { ...state, notice: 'Select at least one message first' }
      return {
        ...state,
        mailbox: applyKind(state.mailbox, action.ids, action.action),
        undo: { previous: state.mailbox, ids: action.ids },
        notice: applyNotice[action.action],
      }
    case 'move-to':
      if (!action.ids.length) return { ...state, notice: 'Select at least one message first' }
      return {
        ...state,
        mailbox: state.mailbox.map(mail => atIds(action.ids, mail) ? { ...mail, folder: action.folder } : mail),
        undo: { previous: state.mailbox, ids: action.ids },
        notice: `Moved to ${action.folder}`,
      }
    case 'empty-trash':
      return { ...state, mailbox: state.mailbox.filter(mail => mail.folder !== 'Trash'), notice: 'Trash emptied' }
    case 'snooze':
      if (!action.ids.length) return { ...state, notice: 'Select at least one message first' }
      return {
        ...state,
        mailbox: state.mailbox.map(mail => atIds(action.ids, mail) ? { ...mail, folder: 'Snoozed', snoozedUntil: action.until } : mail),
        undo: { previous: state.mailbox, ids: action.ids },
        notice: `Snoozed until ${new Date(action.until).toLocaleString([], { month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit' })}`,
      }
    case 'unsnooze':
      return {
        ...state,
        mailbox: state.mailbox.map(mail =>
          mail.folder === 'Snoozed' && mail.snoozedUntil && new Date(mail.snoozedUntil).getTime() <= Date.now()
            ? { ...mail, folder: 'Inbox', snoozedUntil: undefined }
            : mail,
        ),
      }
    case 'undo':
      return state.undo ? { ...state, mailbox: state.undo.previous, undo: null, notice: 'Action undone' } : state
    case 'too-late':
      return { ...state, undo: null }
    case 'sent':
      return { ...state, mailbox: [action.mail, ...state.mailbox], notice: 'Message sent' }
    case 'receive':
      return { ...state, mailbox: [action.mail, ...state.mailbox] }
    case 'unsend-mail':
      return { ...state, mailbox: state.mailbox.filter(mail => mail.id !== action.id), notice: 'Send undone' }
    case 'notice':
      return { ...state, notice: action.message }
    case 'clear-notice':
      return { ...state, notice: '' }
  }
}