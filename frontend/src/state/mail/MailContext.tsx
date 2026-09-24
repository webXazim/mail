import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useReducer,
  useRef,
  useState,
  type ReactNode,
} from 'react'
import { buildSentMail, parseAddresses } from '../../lib/mail'
import { applyFaviconBadge } from '../../lib/favicon'
import { ApiError, isSessionActive } from '../../lib/api'
import { buildReplyMail } from '../../lib/delivery'
import { applyIncomingFilters } from '../../lib/pipeline'
import { mailboxApi } from '../../services/mailbox'
import {
  emailMailboxFor,
  emptyTrashRemote,
  fetchMailboxes,
  isRemoteMail,
  loadMailboxMail,
  moveEmails,
  parseRecipients,
  remoteDraftApi,
  resetMailCache,
  sendCompose,
  setRead,
  setStarred,
  type RemoteMailbox,
} from '../../services/remote-mail'
import { accountsApi, unifiedViewId, primaryAccountId, type Account } from '../../services/accounts'
import { notificationsApi } from '../../services/notifications'
import { contactsService } from '../../services/contacts'
import { scheduleApi } from '../../services/schedule'
import { settingsApi } from '../../services/settings'
import { profileApi } from '../../services/profile'
import { receiptRequestsApi } from '../../services/receipts'
import type { RealtimeEvent } from '../../services/ws'
import type { Draft, Mail, Mailbox } from '../../types'
import { initialMailState, mailboxReducer, type MailActionKind } from './mailboxReducer'

export type MailToast = {
  id: number
  message: string
  canUndoAction: boolean
  canUndoSend: boolean
}

const playAlertBeep = () => {
  try {
    const context = new AudioContext()
    const oscillator = context.createOscillator()
    const gain = context.createGain()
    oscillator.frequency.value = 880
    gain.gain.setValueAtTime(0.05, context.currentTime)
    gain.gain.exponentialRampToValueAtTime(0.0001, context.currentTime + 0.15)
    oscillator.connect(gain)
    gain.connect(context.destination)
    oscillator.start()
    oscillator.stop(context.currentTime + 0.16)
    oscillator.onended = () => void context.close()
  } catch {
    /* audio unavailable */
  }
}

const chime = () => {
  if (settingsApi.load().alertSound) playAlertBeep()
}

export type MailContextValue = {
  mailbox: Mail[]
  accounts: Account[]
  activeAccount: string
  setActiveAccount: (accountId: string) => void
  addAccount: (input: { name: string; email: string; password: string }) => Account
  removeAccount: (accountId: string) => void
  loading: boolean
  loadError: boolean
  notice: string
  undoActive: boolean
  undoSendActive: boolean
  undoSecondsLeft: number
  toasts: MailToast[]
  composeOpen: boolean
  composerInitial: Partial<Draft> | null
  scheduledCount: number
  remoteMailboxes: RemoteMailbox[]
  mergeRemoteMails: (mails: Mail[]) => void
  refreshMailboxRoster: () => Promise<RemoteMailbox[]>
  openCompose: (initial?: Partial<Draft>) => void
  closeCompose: () => void
  markRead: (id: string) => void
  markUnread: (ids: string[]) => void
  markAllRead: (ids: string[]) => void
  toggleStar: (ids: string[]) => void
  toggleLabel: (ids: string[], label: string) => void
  applyAction: (action: MailActionKind, ids: string[]) => void
  moveToFolder: (ids: string[], folder: string) => void
  emptyTrash: () => void
  snooze: (ids: string[], until: string) => void
  removeScheduled: (id: string) => void
  undoAction: () => void
  unsend: () => void
  dismissToast: (id: number) => void
  handleSent: (draft: Draft) => Promise<boolean>
  importMails: (mails: Mail[]) => void
  notify: (message: string) => void
  reload: () => void
}

const MailContext = createContext<MailContextValue | null>(null)

export function MailProvider({ children }: { children: ReactNode }) {
  const [state, dispatch] = useReducer(mailboxReducer, initialMailState)
  const [accounts, setAccounts] = useState<Account[]>(() => accountsApi.list())
  const [activeAccount, setActiveAccountState] = useState<string>(unifiedViewId)
  const [composeOpen, setComposeOpen] = useState(false)
  const [composerInitial, setComposerInitial] = useState<Partial<Draft> | null>(null)
  const [scheduledCount, setScheduledCount] = useState(() => scheduleApi.list().length)
  const [remoteMailboxes, setRemoteMailboxes] = useState<RemoteMailbox[]>([])
  const [undoSendActive, setUndoSendActive] = useState(false)
  const [undoSecondsLeft, setUndoSendSecondsLeft] = useState(0)
  const sentIdRef = useRef<string | null>(null)
  const replyTimerRef = useRef<number | null>(null)
  const timersRef = useRef<number[]>([])
  const [toasts, setToasts] = useState<MailToast[]>([])
  const toastSeqRef = useRef(0)
  const prevNoticeRef = useRef(state.notice)
  const remoteRef = useRef(isSessionActive() && isRemoteMail())

  const loadMailbox = useCallback(async () => {
    if (isRemoteMail()) {
      if (!isSessionActive()) {
        setRemoteMailboxes([])
        dispatch({ type: 'hydrated', mailbox: [] })
        return
      }
      try {
        const profile = await profileApi.refresh()
        if (profile?.has_mailbox === false) {
          setRemoteMailboxes([])
          dispatch({ type: 'hydrated', mailbox: [] })
          return
        }
        const primary = await loadMailboxMail()
        setRemoteMailboxes(await fetchMailboxes())
        // Incoming filtering/forwarding/vacation are server-owned in live
        // sessions. Never mutate the provider snapshot through browser rules.
        dispatch({ type: 'hydrated', mailbox: primary })
        return
      } catch {
        resetMailCache()
        setRemoteMailboxes([])
        dispatch({ type: 'load-failed' })
        return
      }
    }
    try {
      const primary = await mailboxApi.list()
      const secondaryAccounts = accountsApi
        .list()
        .filter((account) => account.id !== primaryAccountId)
      const secondary = (
        await Promise.all(secondaryAccounts.map((account) => mailboxApi.listFor(account.id)))
      ).flat()
      const combined = secondary.length ? [...primary, ...secondary] : primary
      const applied = applyIncomingFilters(combined)
      dispatch({ type: 'hydrated', mailbox: applied.mailbox })
      if (applied.report.forwarded.length) {
        dispatch({
          type: 'notice',
          message: applied.report.forwarded
            .map(
              (item) =>
                `Forwarded ${item.count} ${item.count === 1 ? 'message' : 'messages'} to ${item.address}`,
            )
            .join(' · '),
        })
      }
    } catch {
      dispatch({ type: 'load-failed' })
    }
  }, [])

  useEffect(() => {
    void loadMailbox()
  }, [loadMailbox])

  useEffect(() => {
    const syncAuth = () => {
      remoteRef.current = isSessionActive() && isRemoteMail()
      dispatch({ type: 'retry' })
      void loadMailbox()
      if (remoteRef.current) void profileApi.refresh().then(() => setAccounts(accountsApi.list())).catch(() => {})
      else setAccounts(accountsApi.list())
    }
    window.addEventListener('cs-mail-auth-changed', syncAuth)
    return () => window.removeEventListener('cs-mail-auth-changed', syncAuth)
  }, [loadMailbox])

  // Bootstrap the real identity (name/email) so the account switcher and
  // composer stop showing the demo fixture for signed-in users.
  useEffect(() => {
    if (!remoteRef.current || !isRemoteMail()) return
    void profileApi
      .refresh()
      .then((profile) => {
        if (profile) setAccounts(accountsApi.list())
      })
      .catch(() => {})
  }, [])

  const refreshMailboxRoster = useCallback(async () => {
    if (!remoteRef.current || !isRemoteMail()) return []
    const list = await fetchMailboxes(true)
    setRemoteMailboxes(list)
    return list
  }, [])

  const mergeRemoteMails = useCallback((mails: Mail[]) => {
    dispatch({ type: 'upsert', mails })
  }, [])

  useEffect(() => {
    if (!remoteRef.current || !isRemoteMail()) return
    const refresh = () => { void refreshMailboxRoster().catch(() => {}) }
    window.addEventListener('cs-mail-folders-changed', refresh)
    return () => window.removeEventListener('cs-mail-folders-changed', refresh)
  }, [refreshMailboxRoster])

  useEffect(() => {
    const onRealtime = (incoming: Event) => {
      const detail = (incoming as CustomEvent<RealtimeEvent>).detail
      if (!detail || !remoteRef.current) return
      if (detail.kind === 'new-mail') {
        chime()
        return
      }
      if (detail.kind !== 'resource-changed') return
      switch (detail.payload.resource) {
        case 'mailbox':
          void loadMailbox()
          break
        case 'schedule':
          void scheduleApi.refresh().then((items) => setScheduledCount(items.length)).catch(() => {})
          break
        case 'profile':
          void profileApi.refresh().then(() => setAccounts(accountsApi.list())).catch(() => {})
          break
      }
    }
    window.addEventListener('cs-mail-realtime', onRealtime)
    return () => window.removeEventListener('cs-mail-realtime', onRealtime)
  }, [loadMailbox])

  const reload = useCallback(() => {
    dispatch({ type: 'retry' })
    void loadMailbox()
  }, [loadMailbox])

  useEffect(() => {
    if (state.loading) return
    const fireScheduled = () => {
      dispatch({ type: 'unsnooze' })
      scheduleApi.dueItems().forEach((item) => {
        const time = new Date(item.at).toLocaleTimeString([], {
          hour: 'numeric',
          minute: '2-digit',
        })
        dispatch({ type: 'sent', mail: buildSentMail(item.draft, time) })
        void scheduleApi.remove(item.id)
        chime()
      })
      setScheduledCount(scheduleApi.list().length)
    }
    const refreshScheduled = () => {
      if (!remoteRef.current || !isRemoteMail()) return
      void scheduleApi
        .refresh()
        .then((list) => setScheduledCount(list.length))
        .catch(() => {})
    }
    fireScheduled()
    refreshScheduled()
    const interval = window.setInterval(() => {
      fireScheduled()
      refreshScheduled()
    }, 30000)
    return () => window.clearInterval(interval)
  }, [state.loading])

  useEffect(() => {
    if (state.loading || remoteRef.current) return
    const timer = window.setTimeout(() => {
      const all = state.mailbox
      void mailboxApi
        .replace(all.filter((mail) => (mail.accountId ?? primaryAccountId) === primaryAccountId))
        .catch(() => dispatch({ type: 'notice', message: 'Unable to save mailbox changes' }))
      accountsApi
        .list()
        .filter((account) => account.id !== primaryAccountId)
        .forEach((account) => {
          void mailboxApi
            .replaceFor(
              account.id,
              all.filter((mail) => mail.accountId === account.id),
            )
            .catch(() => {
              /* best effort */
            })
        })
    }, 400)
    return () => window.clearTimeout(timer)
  }, [state.mailbox, state.loading])

  useEffect(
    () => () => {
      timersRef.current.forEach(window.clearTimeout)
      if (replyTimerRef.current !== null) window.clearTimeout(replyTimerRef.current)
    },
    [],
  )

  useEffect(() => {
    const settings = settingsApi.load()
    const trashId = remoteMailboxes.find((mailbox) => mailbox.role === 'trash')?.id
    const unread = remoteRef.current && remoteMailboxes.length
      ? remoteMailboxes
          .filter((mailbox) => mailbox.id !== trashId)
          .reduce((sum, mailbox) => sum + mailbox.unread, 0)
      : state.mailbox.filter((mail) => mail.unread && mail.folder !== 'Trash').length
    document.title = settings.unreadBadge && unread ? `(${unread}) CS Mail` : 'CS Mail'
    applyFaviconBadge(settings.unreadBadge ? unread : 0)
  }, [state.mailbox, remoteMailboxes])

  useEffect(() => {
    if (!undoSendActive) return
    const interval = window.setInterval(
      () => setUndoSendSecondsLeft((current) => Math.max(0, current - 1)),
      1000,
    )
    return () => window.clearInterval(interval)
  }, [undoSendActive])

  useEffect(() => {
    if (state.notice && state.notice !== prevNoticeRef.current) {
      const id = ++toastSeqRef.current
      setToasts((current) => [
        ...current,
        {
          id,
          message: state.notice,
          canUndoAction: state.undo !== null,
          canUndoSend: undoSendActive,
        },
      ])
      timersRef.current.push(
        window.setTimeout(
          () => setToasts((current) => current.filter((toast) => toast.id !== id)),
          7000,
        ),
      )
    }
    prevNoticeRef.current = state.notice
  }, [state.notice, state.undo, undoSendActive])

  const dismissToast = useCallback(
    (id: number) => setToasts((current) => current.filter((toast) => toast.id !== id)),
    [],
  )
  const notify = useCallback((message: string) => dispatch({ type: 'notice', message }), [])

  const pushDesktopNotification = useCallback((title: string, body: string) => {
    try {
      if (!settingsApi.load().desktopNotifications) return
      if (typeof Notification === 'undefined' || Notification.permission !== 'granted') return
      const notification = new Notification(title, { body, tag: `cs-mail-${Date.now()}` })
      notification.onclick = () => window.focus()
    } catch {
      /* notifications unavailable */
    }
  }, [])

  const arrive = useCallback(
    (mail: Mail) => {
      const { mailbox: processedMail, report } = applyIncomingFilters([mail])
      const processed = processedMail[0] ?? mail
      dispatch({ type: 'receive', mail: processed })
      if (report.forwarded.length) {
        const item = report.forwarded[0]
        dispatch({
          type: 'notice',
          message: `Forwarded ${item.count} ${item.count === 1 ? 'message' : 'messages'} to ${item.address}`,
        })
      } else if (processed.folder === 'Trash') {
        dispatch({ type: 'notice', message: `Discarded an unwanted message from ${mail.sender}` })
      } else {
        dispatch({ type: 'notice', message: `New mail from ${mail.sender} — ${mail.subject}` })
      }
      if (processed.folder === 'Trash') return
      chime()
      pushDesktopNotification(mail.sender, mail.subject)
      notificationsApi.add({
        icon: 'mail',
        title: `New mail from ${mail.sender}`,
        detail: mail.subject,
      })
      if (mail.email && !mail.email.toLowerCase().endsWith('@crescentsphere.com')) {
        void contactsService.upsert({ name: mail.sender, email: mail.email.toLowerCase() })
      }
    },
    [pushDesktopNotification],
  )

  const openCompose = useCallback((initial?: Partial<Draft>) => {
    setComposerInitial(initial ?? null)
    setComposeOpen(true)
  }, [])
  const closeCompose = useCallback(() => {
    setComposeOpen(false)
    setComposerInitial(null)
  }, [])

  const markRead = useCallback((id: string) => {
    dispatch({ type: 'mark-read', ids: [id] })
    if (remoteRef.current) void setRead([id], true).then(refreshMailboxRoster).catch(() => {})
  }, [refreshMailboxRoster])
  const markUnread = useCallback((ids: string[]) => {
    dispatch({ type: 'mark-unread', ids })
    if (remoteRef.current) void setRead(ids, false).then(refreshMailboxRoster).catch(() => {})
  }, [refreshMailboxRoster])
  const markAllRead = useCallback((ids: string[]) => {
    dispatch({ type: 'mark-all-read', ids })
    if (remoteRef.current) void setRead(ids, true).then(refreshMailboxRoster).catch(() => {})
  }, [refreshMailboxRoster])
  const toggleStar = useCallback(
    (ids: string[]) => {
      const starred = !ids.every((id) => state.mailbox.find((mail) => mail.id === id)?.starred)
      dispatch({ type: 'toggle-star', ids })
      if (remoteRef.current) void setStarred(ids, starred).then(refreshMailboxRoster).catch(() => {})
    },
    [state.mailbox, refreshMailboxRoster],
  )
  const toggleLabel = useCallback(
    (ids: string[], label: string) => dispatch({ type: 'toggle-label', ids, label }),
    [],
  )
  const moveToFolder = useCallback((ids: string[], folder: string) => {
    dispatch({ type: 'move-to', ids, folder })
    if (!remoteRef.current) return
    const fromMailboxes = Object.fromEntries(
      ids.map((id) => [id, emailMailboxFor(id)] as const).filter((entry): entry is readonly [string, string] => Boolean(entry[1])),
    )
    void moveEmails(ids, folder, fromMailboxes).then(refreshMailboxRoster).catch(() => {})
  }, [refreshMailboxRoster])
  const emptyTrash = useCallback(() => {
    dispatch({ type: 'empty-trash' })
    if (remoteRef.current) void emptyTrashRemote().then(refreshMailboxRoster).catch(() => {})
  }, [refreshMailboxRoster])
  const snooze = useCallback(
    (ids: string[], until: string) => dispatch({ type: 'snooze', ids, until }),
    [],
  )
  const removeScheduled = useCallback((id: string) => {
    void scheduleApi
      .remove(id)
      .then(() => {
        setScheduledCount(scheduleApi.list().length)
        dispatch({ type: 'notice', message: 'Scheduled message canceled' })
      })
      .catch((error) => {
        dispatch({
          type: 'notice',
          message: error instanceof Error ? error.message : 'Could not cancel scheduled message',
        })
      })
  }, [])

  const applyAction = useCallback((action: MailActionKind, ids: string[]) => {
    dispatch({ type: 'apply', ids, action })
    if (remoteRef.current) {
      if (action === 'read') {
        void setRead(ids, true).then(refreshMailboxRoster).catch(() => {})
      } else if (action === 'archive' || action === 'trash') {
        const target = action === 'archive' ? 'Archive' : 'Trash'
        const fromMailboxes = Object.fromEntries(
          ids.map((id) => [id, emailMailboxFor(id)] as const).filter((entry): entry is readonly [string, string] => Boolean(entry[1])),
        )
        void moveEmails(ids, target, fromMailboxes).then(refreshMailboxRoster).catch(() => {})
      }
    }
    timersRef.current.push(
      window.setTimeout(() => {
        dispatch({ type: 'clear-notice' })
        dispatch({ type: 'too-late' })
      }, 5000),
    )
  }, [refreshMailboxRoster])

  const undoAction = useCallback(() => {
    dispatch({ type: 'undo' })
    timersRef.current.push(window.setTimeout(() => dispatch({ type: 'clear-notice' }), 1800))
  }, [])

  const handleSent = useCallback(
    async (draft: Draft): Promise<boolean> => {
      if (draft.scheduledAt) {
        const at = new Date(draft.scheduledAt)
        if (Number.isNaN(at.getTime())) {
          dispatch({ type: 'notice', message: 'Choose a valid date and time to schedule' })
          return false
        }
        if (
          remoteRef.current &&
          isRemoteMail() &&
          draft.attachments.some((attachment) => attachment.id.startsWith('local-'))
        ) {
          dispatch({
            type: 'notice',
            message: 'One or more attachments is not uploaded — re-add it before scheduling',
          })
          return false
        }
        try {
          const sourceDraftId = remoteRef.current && isRemoteMail() ? remoteDraftApi.activeId() : null
          await scheduleApi.enqueue({
            id: `scheduled-${Date.now()}`,
            draft,
            at: at.toISOString(),
          })
          if (sourceDraftId) {
            // The scheduled row now owns its attachment references. Removing
            // the source draft avoids a second stale copy in Drafts.
            await remoteDraftApi.remove(sourceDraftId).catch(() => {})
          }
          setScheduledCount(scheduleApi.list().length)
          dispatch({
            type: 'notice',
            message: `Scheduled for ${at.toLocaleString([], { month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit' })}`,
          })
          return true
        } catch (error) {
          dispatch({
            type: 'notice',
            message: `Schedule failed — ${error instanceof Error ? error.message : 'please try again'}`,
          })
          return false
        }
      }

      if (remoteRef.current && isRemoteMail()) {
        if (draft.attachments.some((attachment) => attachment.id.startsWith('local-'))) {
          dispatch({
            type: 'notice',
            message: 'One or more attachments is not uploaded — re-add it before sending',
          })
          return false
        }
        const compose = {
          to: parseRecipients(draft.to),
          cc: parseRecipients(draft.cc),
          bcc: parseRecipients(draft.bcc),
          subject: draft.subject,
          body_text: draft.body,
          attachments: draft.attachments.map((attachment) => ({ id: attachment.id })),
          identity_id: draft.identityId,
          client_key: draft.clientKey,
          send_key: draft.sendKey,
        }
        try {
          const outcome = await sendCompose(
            compose,
            draft.sendKey || crypto.randomUUID(),
            remoteDraftApi.activeId(),
          )
          chime()
          dispatch({
            type: 'notice',
            message: outcome.stored
              ? 'Sent — a copy is on its way to your Sent folder'
              : 'Sent — delivery may take a few seconds',
          })
          window.setTimeout(() => {
            void loadMailbox()
          }, 1500)
          return true
        } catch (error) {
          const uncertain =
            error instanceof TypeError ||
            (error instanceof ApiError &&
              (error.code === 'send_uncertain' || error.code === 'send_in_progress'))
          dispatch({
            type: 'notice',
            message: uncertain
              ? 'Send status is being reconciled — do not create a duplicate. Press Send again to safely check the same attempt.'
              : `Send failed — ${error instanceof Error ? error.message : 'please try again'}`,
          })
          return false
        }
      }

      const mail = buildSentMail(draft)
      dispatch({ type: 'sent', mail })
      chime()
      sentIdRef.current = mail.id
      setUndoSendActive(true)
      setUndoSendSecondsLeft(5)
      timersRef.current.push(
        window.setTimeout(() => {
          sentIdRef.current = null
          setUndoSendActive(false)
        }, 5000),
      )
      const inboxRecipients =
        (draft.to + ',' + draft.cc).match(/[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}/g) ?? []
      const firstRecipient = inboxRecipients[0]?.toLowerCase()
      if (draft.receiptRequested && firstRecipient)
        void receiptRequestsApi.request(mail.id, firstRecipient)
      if (firstRecipient && !firstRecipient.endsWith('@crescentsphere.com')) {
        const contact = contactsService
          .list()
          .find((item) => item.email.toLowerCase() === firstRecipient)
        replyTimerRef.current = window.setTimeout(
          () => arrive(buildReplyMail(draft, contact?.name)),
          8000,
        )
      }
      parseAddresses(draft.to + ',' + draft.cc).forEach((part) => {
        const displayName = part.match(/^([^<@]+?)\s*<[^>]+>$/)?.[1]?.trim()
        const email = part.includes('<')
          ? (part.match(/<([^>]+)>/)?.[1] ?? part).toLowerCase()
          : part.toLowerCase()
        if (/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email)) {
          void contactsService.upsert({ name: displayName || email.split('@')[0], email })
        }
      })
      return true
    },
    [arrive, loadMailbox],
  )

  const importMails = useCallback((mails: Mail[]) => {
    mails.forEach((mail) => dispatch({ type: 'receive', mail }))
  }, [])

  const unsend = useCallback(() => {
    if (!sentIdRef.current) return
    if (replyTimerRef.current !== null) {
      window.clearTimeout(replyTimerRef.current)
      replyTimerRef.current = null
    }
    dispatch({ type: 'unsend-mail', id: sentIdRef.current })
    sentIdRef.current = null
    setUndoSendActive(false)
  }, [])

  const mailbox = useMemo(
    () =>
      activeAccount === unifiedViewId
        ? state.mailbox
        : state.mailbox.filter((mail) => (mail.accountId ?? primaryAccountId) === activeAccount),
    [state.mailbox, activeAccount],
  )

  const setActiveAccount = useCallback((accountId: string) => {
    setActiveAccountState(accountId)
    dispatch({ type: 'clear-notice' })
  }, [])

  const addAccount = useCallback((input: { name: string; email: string; password: string }) => {
    const result = accountsApi.add(input)
    setAccounts(accountsApi.list())
    dispatch({ type: 'append', mails: result.mailbox })
    dispatch({ type: 'notice', message: `Added ${result.account.email} to your accounts` })
    return result.account
  }, [])

  const removeAccount = useCallback(
    (accountId: string) => {
      accountsApi.remove(accountId)
      setAccounts(accountsApi.list())
      if (activeAccount === accountId) setActiveAccountState(unifiedViewId)
      dispatch({ type: 'drop-account', accountId })
      dispatch({ type: 'notice', message: 'Account removed' })
    },
    [activeAccount],
  )

  const value = useMemo<MailContextValue>(
    () => ({
      mailbox,
      accounts,
      activeAccount,
      setActiveAccount,
      addAccount,
      removeAccount,
      loading: state.loading,
      loadError: state.loadError,
      notice: state.notice,
      undoActive: state.undo !== null,
      undoSendActive,
      undoSecondsLeft,
      toasts,
      composeOpen,
      composerInitial,
      scheduledCount,
      remoteMailboxes,
      mergeRemoteMails,
      refreshMailboxRoster,
      openCompose,
      closeCompose,
      markRead,
      markUnread,
      markAllRead,
      toggleStar,
      toggleLabel,
      applyAction,
      moveToFolder,

      emptyTrash,
      snooze,
      removeScheduled,
      undoAction,
      unsend,
      dismissToast,
      handleSent,
      importMails,
      notify,
      reload,
    }),
    [
      mailbox,
      accounts,
      activeAccount,
      setActiveAccount,
      addAccount,
      removeAccount,
      state.loading,
      state.loadError,
      state.notice,
      state.undo,
      undoSendActive,
      undoSecondsLeft,
      toasts,
      composeOpen,
      composerInitial,
      scheduledCount,
      remoteMailboxes,
      mergeRemoteMails,
      refreshMailboxRoster,
      openCompose,
      closeCompose,
      markRead,
      markUnread,
      markAllRead,
      toggleStar,
      toggleLabel,
      applyAction,
      moveToFolder,
      emptyTrash,
      snooze,
      removeScheduled,
      undoAction,
      unsend,
      dismissToast,
      handleSent,
      importMails,
      notify,
      reload,
    ],
  )

  return <MailContext.Provider value={value}>{children}</MailContext.Provider>
}

export function useMail(): MailContextValue {
  const context = useContext(MailContext)
  if (!context) throw new Error('useMail must be used within a MailProvider')
  return context
}
