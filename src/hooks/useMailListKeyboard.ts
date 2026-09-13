import { useEffect, useRef, useState } from 'react'
import type { Mail } from '../types'

type Options = {
  items: Mail[]
  readOnly?: boolean
  onOpen: (mail: Mail) => void
  onArchive: (mail: Mail) => void
  onTrash: (mail: Mail) => void
  onStar: (mail: Mail) => void
  onSelect: (mail: Mail) => void
  onUndo: () => void
}

const isEditableTarget = (target: EventTarget | null) =>
  target instanceof HTMLElement &&
  Boolean(target.closest('input, textarea, select, [contenteditable="true"]'))

export function useMailListKeyboard({
  items,
  readOnly = false,
  onOpen,
  onArchive,
  onTrash,
  onStar,
  onSelect,
  onUndo,
}: Options) {
  const [cursor, setCursor] = useState(0)
  const cursorRef = useRef(0)
  const itemsRef = useRef(items)
  const handlersRef = useRef({ onOpen, onArchive, onTrash, onStar, onSelect, onUndo })
  const lastGRef = useRef(0)

  useEffect(() => {
    cursorRef.current = cursor
  }, [cursor])

  useEffect(() => {
    itemsRef.current = items
  }, [items])

  useEffect(() => {
    handlersRef.current = { onOpen, onArchive, onTrash, onStar, onSelect, onUndo }
  }, [onOpen, onArchive, onTrash, onStar, onSelect, onUndo])

  const focused = items.length === 0 ? -1 : cursor < 0 ? -1 : Math.min(cursor, items.length - 1)

  useEffect(() => {
    const handle = (event: KeyboardEvent) => {
      if (isEditableTarget(event.target)) return
      if (event.ctrlKey || event.metaKey || event.altKey) return
      const list = itemsRef.current
      const {
        onOpen: open,
        onArchive: archive,
        onTrash: trash,
        onStar: star,
        onSelect: select,
        onUndo: undo,
      } = handlersRef.current
      const current = Math.max(0, Math.min(cursorRef.current, list.length - 1))
      const key = event.key.toLowerCase()

      if (key === 'g') {
        lastGRef.current = Date.now()
        return
      }
      const gSequenceActive = Date.now() - lastGRef.current < 1500

      const service = (mail: Mail, action: (mail: Mail) => void) => {
        if (readOnly || gSequenceActive) return
        action(mail)
      }

      const move = (delta: number) => {
        if (!list.length) return
        event.preventDefault()
        const next = Math.max(0, Math.min(list.length - 1, current + delta))
        setCursor(next)
      }

      switch (key) {
        case 'j':
          move(1)
          break
        case 'k':
          move(-1)
          break
        case 'enter':
          if (gSequenceActive) break
          if (list[current]) {
            event.preventDefault()
            open(list[current])
          }
          break
        case 'e':
          if (list[current]) service(list[current], archive)
          break
        case '#':
          if (list[current]) service(list[current], trash)
          break
        case 's':
          if (list[current]) service(list[current], star)
          break
        case 'x':
          if (list[current]) service(list[current], select)
          break
        case 'z':
          if (!gSequenceActive) {
            event.preventDefault()
            undo()
          }
          break
        default:
          break
      }
    }
    window.addEventListener('keydown', handle)
    return () => window.removeEventListener('keydown', handle)
  }, [readOnly])

  return { cursor: focused, setCursor }
}
