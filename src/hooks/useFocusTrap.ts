import { useEffect, type RefObject } from 'react'

const FOCUSABLE = 'button, input, textarea, select, [href], [tabindex], [contenteditable="true"]'

const getFocusable = (root: HTMLElement): HTMLElement[] =>
  Array.from(root.querySelectorAll<HTMLElement>(FOCUSABLE)).filter(
    element => !element.hasAttribute('disabled') && element.getAttribute('tabindex') !== '-1',
  )

export function useFocusTrap(target: RefObject<HTMLElement | null>, active = true) {
  useEffect(() => {
    if (!active) return
    const root = target.current
    if (!root) return
    const previouslyFocused = document.activeElement instanceof HTMLElement ? document.activeElement : null
    getFocusable(root)[0]?.focus()
    const trap = (event: KeyboardEvent) => {
      if (event.key !== 'Tab') return
      const elements = getFocusable(root)
      if (!elements.length) return
      const first = elements[0]
      const last = elements[elements.length - 1]
      if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last.focus() }
      else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first.focus() }
    }
    document.addEventListener('keydown', trap)
    return () => {
      document.removeEventListener('keydown', trap)
      previouslyFocused?.focus()
    }
  }, [active, target])
}