import { createContext, useCallback, useContext, useEffect, useMemo, useReducer, useRef, useState, type ReactNode } from 'react'
import { buildSentMail } from '../../lib/mail'
import { applyIncomingFilters } from '../../lib/pipeline'
import { mailboxApi } from '../../services/mailbox'
import { scheduleApi } from '../../services/schedule'
import { settingsApi } from '../../services/settings'
import type { Draft, Mail } from '../../types'
import { initialMailState, mailboxReducer, type MailActionKind } from './mailboxReducer'

export type MailToast = { id: number; message: string; canUndoAction: boolean; canUndoSend: boolean }

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

const chime = () => { if (settingsApi.load().alertSound) playAlertBeep() }

export type MailContextValue = {
  mailbox: Mail[]
  loading: boolean
  loadError: boolean
  notice: string
  undoActive: boolean
  undoSendActive: boolean
  toasts: MailToast[]
  composeOpen: boolean
  composerInitial: Partial<Draft> | null
  scheduledCount: number
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
  handleSent: (draft: Draft) => void
  reload: () => void
}

const MailContext = createContext<MailContextValue | null>(null)

export function MailProvider({ children }: { children: ReactNode }) {
  const [state, dispatch] = useReducer(mailboxReducer, initialMailState)
  const [composeOpen, setComposeOpen] = useState(false)
  const [composerInitial, setComposerInitial] = useState<Partial<Draft> | null>(null)
  const [scheduledCount, setScheduledCount] = useState(() => scheduleApi.list().length)
  const [undoSendActive, setUndoSendActive] = useState(false)
  const sentIdRef = useRef<string | null>(null)
  const timersRef = useRef<number[]>([])
  const [toasts, setToasts] = useState<MailToast[]>([])
  const toastSeqRef = useRef(0)
  const prevNoticeRef = useRef(state.notice)

  const loadMailbox = useCallback(async () => {
    let next
    try {
      next = await mailboxApi.list()
    } catch {
      dispatch({ type: 'load-failed' })
      return
    }
    const applied = applyIncomingFilters(next)
    dispatch({ type: 'hydrated', mailbox: applied.mailbox })
    if (applied.report.forwarded.length) {
      dispatch({ type: 'notice', message: applied.report.forwarded.map(item => `Forwarded ${item.count} ${item.count === 1 ? 'message' : 'messages'} to ${item.address}`).join(' · ') })
    }
  }, [])

  useEffect(() => {
    let active = true
    mailboxApi
      .list()
      .then(next => {
        if (!active) return
        const applied = applyIncomingFilters(next)
        dispatch({ type: 'hydrated', mailbox: applied.mailbox })
        if (applied.report.forwarded.length) {
          dispatch({ type: 'notice', message: applied.report.forwarded.map(item => `Forwarded ${item.count} ${item.count === 1 ? 'message' : 'messages'} to ${item.address}`).join(' · ') })
        }
      })
      .catch(() => { if (active) dispatch({ type: 'load-failed' }) })
    return () => { active = false }
  }, [])

  const reload = useCallback(() => {
    dispatch({ type: 'retry' })
    void loadMailbox()
  }, [loadMailbox])

  useEffect(() => {
    if (state.loading) return
    const fireScheduled = () => {
      dispatch({ type: 'unsnooze' })
      scheduleApi.dueItems().forEach(item => {
        const time = new Date(item.at).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' })
        dispatch({ type: 'sent', mail: buildSentMail(item.draft, time) })
        scheduleApi.remove(item.id)
        chime()
      })
      setScheduledCount(scheduleApi.list().length)
    }
    fireScheduled()
    const interval = window.setInterval(fireScheduled, 30000)
    return () => window.clearInterval(interval)
  }, [state.loading])

  useEffect(() => {
    if (state.loading) return
    const timer = window.setTimeout(() => {
      void mailboxApi.replace(state.mailbox).catch(() => dispatch({ type: 'notice', message: 'Unable to save mailbox changes' }))
    }, 400)
    return () => window.clearTimeout(timer)
  }, [state.mailbox, state.loading])

  useEffect(() => () => { timersRef.current.forEach(window.clearTimeout) }, [])

  useEffect(() => {
    const settings = settingsApi.load()
    if (!settings.unreadBadge) return
    const unread = state.mailbox.filter(mail => mail.unread && mail.folder !== 'Trash').length
    document.title = unread ? `(${unread}) Harbor Mail` : 'Harbor Mail'
  }, [state.mailbox])

  useEffect(() => {
    if (state.notice && state.notice !== prevNoticeRef.current) {
      const id = ++toastSeqRef.current
      setToasts(current => [...current, { id, message: state.notice, canUndoAction: state.undo !== null, canUndoSend: undoSendActive }])
      timersRef.current.push(window.setTimeout(() => setToasts(current => current.filter(toast => toast.id !== id)), 7000))
    }
    prevNoticeRef.current = state.notice
  }, [state.notice, state.undo, undoSendActive])

  const dismissToast = useCallback((id: number) => setToasts(current => current.filter(toast => toast.id !== id)), [])

  const openCompose = useCallback((initial?: Partial<Draft>) => {
    setComposerInitial(initial ?? null)
    setComposeOpen(true)
  }, [])
  const closeCompose = useCallback(() => { setComposeOpen(false); setComposerInitial(null) }, [])

  const markRead = useCallback((id: string) => dispatch({ type: 'mark-read', ids: [id] }), [])
  const markUnread = useCallback((ids: string[]) => dispatch({ type: 'mark-unread', ids }), [])
  const markAllRead = useCallback((ids: string[]) => dispatch({ type: 'mark-all-read', ids }), [])
  const toggleStar = useCallback((ids: string[]) => dispatch({ type: 'toggle-star', ids }), [])
  const toggleLabel = useCallback((ids: string[], label: string) => dispatch({ type: 'toggle-label', ids, label }), [])
  const moveToFolder = useCallback((ids: string[], folder: string) => dispatch({ type: 'move-to', ids, folder }), [])
  const emptyTrash = useCallback(() => dispatch({ type: 'empty-trash' }), [])
  const snooze = useCallback((ids: string[], until: string) => dispatch({ type: 'snooze', ids, until }), [])
  const removeScheduled = useCallback((id: string) => {
    scheduleApi.remove(id)
    setScheduledCount(scheduleApi.list().length)
    dispatch({ type: 'notice', message: 'Scheduled message canceled' })
  }, [])

  const applyAction = useCallback((action: MailActionKind, ids: string[]) => {
    dispatch({ type: 'apply', ids, action })
    timersRef.current.push(window.setTimeout(() => {
      dispatch({ type: 'clear-notice' })
      dispatch({ type: 'too-late' })
    }, 5000))
  }, [])

  const undoAction = useCallback(() => {
    dispatch({ type: 'undo' })
    timersRef.current.push(window.setTimeout(() => dispatch({ type: 'clear-notice' }), 1800))
  }, [])

  const handleSent = useCallback((draft: Draft) => {
    if (draft.scheduledAt) {
      const at = new Date(draft.scheduledAt)
      if (Number.isNaN(at.getTime())) { dispatch({ type: 'notice', message: 'Choose a valid date and time to schedule' }); return }
      scheduleApi.enqueue({ id: `scheduled-${Date.now()}`, draft, at: at.toISOString() })
      setScheduledCount(scheduleApi.list().length)
      dispatch({ type: 'notice', message: `Scheduled for ${at.toLocaleString([], { month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit' })}` })
      return
    }
    const mail = buildSentMail(draft)
    dispatch({ type: 'sent', mail })
    chime()
    sentIdRef.current = mail.id
    setUndoSendActive(true)
    timersRef.current.push(window.setTimeout(() => {
      sentIdRef.current = null
      setUndoSendActive(false)
    }, 5000))
  }, [])

  const unsend = useCallback(() => {
    if (!sentIdRef.current) return
    dispatch({ type: 'unsend-mail', id: sentIdRef.current })
    sentIdRef.current = null
    setUndoSendActive(false)
  }, [])

  const value = useMemo<MailContextValue>(() => ({
    mailbox: state.mailbox,
    loading: state.loading,
    loadError: state.loadError,
    notice: state.notice,
    undoActive: state.undo !== null,
    undoSendActive,
    toasts,
    composeOpen,
    composerInitial,
    scheduledCount,
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
    reload,
  }), [state.mailbox, state.loading, state.loadError, state.notice, state.undo, undoSendActive, toasts, composeOpen, composerInitial, scheduledCount, openCompose, closeCompose, markRead, markUnread, markAllRead, toggleStar, toggleLabel, applyAction, moveToFolder, emptyTrash, snooze, removeScheduled, undoAction, unsend, dismissToast, handleSent, reload])

  return <MailContext.Provider value={value}>{children}</MailContext.Provider>
}

export function useMail(): MailContextValue {
  const context = useContext(MailContext)
  if (!context) throw new Error('useMail must be used within a MailProvider')
  return context
}