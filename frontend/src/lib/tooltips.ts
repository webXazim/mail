let tooltip: HTMLDivElement | null = null
let currentTarget: Element | null = null
let hideTimer: number | null = null

const findCandidate = (target: EventTarget | null): HTMLElement | null => {
  if (!(target instanceof Element)) return null
  return target.closest<HTMLElement>(
    '[data-tooltip], button[aria-label], [role="button"][aria-label]',
  )
}

const visibleLabel = (el: HTMLElement): string | null => {
  if (el.matches('.mail-row__hit, .sidebar-scrim, .compose-layer, .mobile-sheet__scrim'))
    return null
  if ((el as HTMLButtonElement).disabled) return null
  const label = el.dataset.tooltip || el.getAttribute('aria-label') || ''
  const clean = label.trim()
  if (!clean) return null
  if (el.querySelector('[data-tooltip], button[aria-label]')) return null
  const ownText = Array.from(el.childNodes)
    .filter((node) => node.nodeType === Node.TEXT_NODE)
    .map((node) => node.textContent ?? '')
    .join('')
    .trim()
  if (ownText) return null
  return clean
}

const position = (el: HTMLElement, label: string) => {
  if (!tooltip) return
  tooltip.textContent = label
  tooltip.classList.add('is-visible')
  const rect = el.getBoundingClientRect()
  const tipW = tooltip.offsetWidth
  const tipH = tooltip.offsetHeight
  const gap = 8
  let left = rect.left + rect.width / 2 - tipW / 2
  left = Math.max(gap, Math.min(left, window.innerWidth - tipW - gap))
  const placeAbove = rect.top - tipH - gap >= gap
  tooltip.style.left = `${left}px`
  tooltip.style.top = placeAbove ? `${rect.top - tipH - gap}px` : `${rect.bottom + gap}px`
}

const show = (el: HTMLElement) => {
  if (hideTimer !== null) {
    window.clearTimeout(hideTimer)
    hideTimer = null
  }
  const label = visibleLabel(el)
  if (!label) return
  currentTarget = el
  position(el, label)
}

const hide = () => {
  if (hideTimer !== null) window.clearTimeout(hideTimer)
  hideTimer = window.setTimeout(() => {
    if (tooltip) tooltip.classList.remove('is-visible')
    currentTarget = null
  }, 60)
}

export function initTooltips() {
  if (tooltip) return
  if (!document.body) return
  tooltip = document.createElement('div')
  tooltip.className = 'a2t-tooltip'
  tooltip.setAttribute('role', 'tooltip')
  document.body.appendChild(tooltip)

  document.addEventListener('pointerover', (event) => {
    const candidate = findCandidate(event.target)
    if (candidate) show(candidate)
  })
  document.addEventListener('pointerout', (event) => {
    if (event.target instanceof Element && currentTarget?.contains(event.target)) return
    hide()
  })
  const suppress = () => hide()
  window.addEventListener('scroll', suppress, true)
  window.addEventListener('resize', suppress)
  document.addEventListener('click', suppress)
  document.addEventListener('keydown', suppress)
}
