const BASE_FAVICON = '/favicon-32.png?v=2'

let logo: HTMLImageElement | null = null
let currentUnread = 0

function drawFavicon() {
  const link = document.querySelector<HTMLLinkElement>('link[rel="icon"]')
  if (!link) return
  if (!currentUnread || !logo?.complete || !logo.naturalWidth) {
    link.href = BASE_FAVICON
    return
  }

  const canvas = document.createElement('canvas')
  canvas.width = 64
  canvas.height = 64
  const ctx = canvas.getContext('2d')
  if (!ctx) return

  // Preserve the shared CS mark and place the unread count above its edge.
  ctx.drawImage(logo, 0, 0, 64, 64)
  ctx.fillStyle = '#ff6f66'
  ctx.beginPath()
  ctx.arc(50, 14, 12, 0, Math.PI * 2)
  ctx.fill()
  ctx.fillStyle = '#0b0f12'
  ctx.font = 'bold 13px sans-serif'
  ctx.textAlign = 'center'
  ctx.textBaseline = 'middle'
  ctx.fillText(currentUnread > 9 ? '9+' : String(currentUnread), 50, 14)
  link.href = canvas.toDataURL('image/png')
}

export function applyFaviconBadge(unread: number) {
  try {
    if (typeof document === 'undefined') return
    currentUnread = Math.max(0, Math.floor(unread))
    if (!currentUnread) {
      drawFavicon()
      return
    }
    if (!logo) {
      logo = new Image()
      logo.onload = drawFavicon
      logo.onerror = () => { logo = null }
      logo.src = BASE_FAVICON
    }
    drawFavicon()
  } catch {
    /* The favicon badge is decorative. */
  }
}
