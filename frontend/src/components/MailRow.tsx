import { Archive, RotateCcw, Star, Trash2, X } from 'lucide-react'
import { memo, useRef, useState } from 'react'
import { presenceOf } from '../services/presence'
import type { Mail } from '../types'

type Props = {
  mail: Mail
  checked?: boolean
  active?: boolean
  focused?: boolean
  onClick: (mail: Mail) => void
  onSelect?: (checked: boolean, id: string) => void
  onQuickArchive?: (mail: Mail) => void
  onQuickStar?: (mail: Mail) => void
  onQuickTrash?: (mail: Mail) => void
  onQuickCancel?: (mail: Mail) => void
  onQuickRetry?: (mail: Mail) => void
  onLongPress?: (mail: Mail) => void
  swipeable?: boolean
  onSwipeArchive?: (mail: Mail) => void
  onSwipeTrash?: (mail: Mail) => void
}

const SWIPE_W = 152

export const MailRow = memo(function MailRow({
  mail,
  checked = false,
  active = false,
  focused = false,
  onClick,
  onSelect,
  onQuickArchive,
  onQuickStar,
  onQuickTrash,
  onQuickCancel,
  onQuickRetry,
  onLongPress,
  swipeable = false,
  onSwipeArchive,
  onSwipeTrash,
}: Props) {
  const timerRef = useRef<number | null>(null)
  const originRef = useRef<{ x: number; y: number } | null>(null)
  const suppressRef = useRef(false)
  const swipeStartRef = useRef<{ x: number; y: number } | null>(null)
  const swipeDraggingRef = useRef(false)
  const [pressed, setPressed] = useState(false)
  const [swipeOpen, setSwipeOpen] = useState(false)
  const [swipePx, setSwipePx] = useState(0)

  const clearTimer = () => {
    if (timerRef.current !== null) {
      window.clearTimeout(timerRef.current)
      timerRef.current = null
    }
  }

  const isInteractive = (target: EventTarget | null) =>
    target instanceof HTMLElement &&
    Boolean(target.closest('input, select, textarea, .mail-row__quicks, .mail-row__swipe'))

  const closeSwipe = () => {
    setSwipeOpen(false)
    setSwipePx(0)
    swipeDraggingRef.current = false
    swipeStartRef.current = null
  }

  const handleDown = (event: React.PointerEvent<HTMLElement>) => {
    const interactive = isInteractive(event.target)
    if (swipeable && !interactive) {
      swipeStartRef.current = { x: event.clientX, y: event.clientY }
      if (swipeOpen) {
        suppressRef.current = true
        closeSwipe()
        return
      }
    }
    if (event.button !== 0 || !onLongPress || interactive) return
    originRef.current = { x: event.clientX, y: event.clientY }
    suppressRef.current = false
    clearTimer()
    setPressed(true)
    timerRef.current = window.setTimeout(() => {
      timerRef.current = null
      setPressed(false)
      suppressRef.current = true
      onLongPress(mail)
    }, 450)
  }

  const handleMove = (event: React.PointerEvent<HTMLElement>) => {
    if (swipeable && !isInteractive(event.target) && swipeStartRef.current) {
      const dx = event.clientX - swipeStartRef.current.x
      const dy = event.clientY - swipeStartRef.current.y
      if (Math.abs(dx) > Math.abs(dy) && dx < -8) {
        swipeDraggingRef.current = true
        suppressRef.current = true
        clearTimer()
        setPressed(false)
        setSwipePx(Math.max(dx, -SWIPE_W))
        return
      }
      if (swipeDraggingRef.current && dx < 0) {
        setSwipePx(Math.max(dx, -SWIPE_W))
        return
      }
    }
    if (timerRef.current === null || !originRef.current) return
    const deltaX = event.clientX - originRef.current.x
    const deltaY = event.clientY - originRef.current.y
    if (Math.hypot(deltaX, deltaY) > 12) {
      clearTimer()
      setPressed(false)
    }
  }

  const handleEnd = () => {
    if (swipeDraggingRef.current) {
      if (swipePx < -SWIPE_W / 2) {
        setSwipeOpen(true)
        setSwipePx(0)
      } else {
        setSwipeOpen(false)
      }
      swipeDraggingRef.current = false
      swipeStartRef.current = null
      return
    }
    clearTimer()
    originRef.current = null
    swipeStartRef.current = null
    setPressed(false)
  }

  const handleContextMenu = (event: React.MouseEvent<HTMLElement>) => {
    event.preventDefault()
    if (!onLongPress) return
    setPressed(false)
    onLongPress(mail)
  }

  const runSwipeAction = (action: ((mail: Mail) => void) | undefined) => {
    suppressRef.current = true
    action?.(mail)
    closeSwipe()
  }

  const innerTransform = swipeDraggingRef.current
    ? `translateX(${swipePx}px)`
    : swipeOpen
      ? `translateX(-${SWIPE_W}px)`
      : 'translateX(0px)'

  return (
    <article
      className={`mail-row ${mail.unread ? 'mail-row--unread' : ''} ${active ? 'mail-row--active' : ''} ${focused ? 'mail-row--focused' : ''} ${pressed ? 'mail-row--pressed' : ''} ${swipeable ? 'mail-row--swipe' : ''} ${swipeOpen ? 'mail-row--swipe-open' : ''}`}
      aria-current={active ? 'page' : undefined}
      onPointerDown={handleDown}
      onPointerMove={handleMove}
      onPointerUp={handleEnd}
      onPointerCancel={handleEnd}
      onContextMenu={handleContextMenu}
    >
      {swipeable && (
        <span
          className="mail-row__swipe"
          onClick={(event) => {
            event.stopPropagation()
            closeSwipe()
          }}
        >
          {onSwipeArchive && (
            <button
              type="button"
              className="mail-row__swipe-action mail-row__swipe-action--archive"
              aria-label={`Archive ${mail.subject}`}
              tabIndex={swipeOpen ? 0 : -1}
              onClick={(event) => {
                event.stopPropagation()
                runSwipeAction(onSwipeArchive)
              }}
            >
              <Archive size={18} />
              <span>Archive</span>
            </button>
          )}
          {onSwipeTrash && (
            <button
              type="button"
              className="mail-row__swipe-action mail-row__swipe-action--trash"
              aria-label={`Move ${mail.subject} to trash`}
              tabIndex={swipeOpen ? 0 : -1}
              onClick={(event) => {
                event.stopPropagation()
                runSwipeAction(onSwipeTrash)
              }}
            >
              <Trash2 size={18} />
              <span>Delete</span>
            </button>
          )}
        </span>
      )}
      <span
        className="mail-row__inner"
        style={
          swipeable
            ? {
                transform: innerTransform,
                transition: swipeDraggingRef.current ? 'none' : 'transform 0.2s ease',
              }
            : undefined
        }
      >
        <input
          type="checkbox"
          checked={checked}
          disabled={!onSelect}
          onChange={(event) => onSelect?.(event.target.checked, mail.id)}
          onClick={(event) => event.stopPropagation()}
          aria-label={`Select ${mail.subject}`}
        />
        <span className={`avatar avatar--${mail.color}`}>
          {mail.initials}
          {presenceOf(mail.email) !== 'offline' && (
            <i
              className={`presence presence--${presenceOf(mail.email)}`}
              aria-label={`${presenceOf(mail.email)}`}
            />
          )}
        </span>
        <span className="mail-row__sender">
          <strong>{mail.sender}</strong>
          <small>{mail.email}</small>
        </span>
        <span className="mail-row__content">
          <strong>{mail.subject}</strong>
          <small>{mail.preview}</small>
        </span>
        <span className="mail-row__meta">
          <span className="mail-row__quicks" onClick={(event) => event.stopPropagation()}>
            {onQuickRetry && (
              <button
                type="button"
                aria-label={`Retry ${mail.subject}`}
                onClick={() => onQuickRetry(mail)}
              >
                <RotateCcw size={15} />
              </button>
            )}
            {onQuickCancel && (
              <button
                type="button"
                aria-label={`Cancel ${mail.subject}`}
                onClick={() => onQuickCancel(mail)}
              >
                <X size={15} />
              </button>
            )}
            {onQuickArchive && (
              <button
                type="button"
                aria-label={`Archive ${mail.subject}`}
                onClick={() => onQuickArchive(mail)}
              >
                <Archive size={15} />
              </button>
            )}
            {onQuickStar && (
              <button
                type="button"
                aria-label={`${mail.starred ? 'Unstar' : 'Star'} ${mail.subject}`}
                onClick={() => onQuickStar(mail)}
              >
                <Star size={15} fill={mail.starred ? 'currentColor' : 'none'} />
              </button>
            )}
            {onQuickTrash && (
              <button
                type="button"
                aria-label={`Move ${mail.subject} to trash`}
                onClick={() => onQuickTrash(mail)}
              >
                <Trash2 size={15} />
              </button>
            )}
          </span>
          <time>{mail.time}</time>
          {mail.receiptRequested && (
            <span className="badge badge--receipt" title="Read receipt requested">
              Awaiting receipt
            </span>
          )}
          {active && <span className="badge badge--open">Open</span>}
          {mail.label && <span className="badge">{mail.label}</span>}
        </span>
        <button
          type="button"
          className="mail-row__hit"
          aria-label={`Open ${mail.subject}`}
          onClick={() => onClick(mail)}
        />
      </span>
    </article>
  )
})
