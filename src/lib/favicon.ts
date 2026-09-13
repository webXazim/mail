export function applyFaviconBadge(unread: number) {
  try {
    if (typeof document === 'undefined') return
    const link = document.querySelector<HTMLLinkElement>('link[rel="icon"]')
    if (!link || typeof HTMLCanvasElement === 'undefined') return
    const canvas = document.createElement('canvas')
    canvas.width = 64
    canvas.height = 64
    const ctx = canvas.getContext('2d')
    if (!ctx) return
    ctx.fillStyle = '#0b0f12'
    ctx.fillRect(0, 0, 64, 64)
    ctx.strokeStyle = '#c8f34f'
    ctx.lineWidth = 3
    ctx.strokeRect(12, 12, 40, 40)
    ctx.fillStyle = '#c8f34f'
    ctx.fillRect(19, 21, 3, 22)
    ctx.fillRect(42, 21, 3, 22)
    ctx.fillRect(19, 30, 26, 3)
    if (unread > 0) {
      ctx.fillStyle = '#ff6f66'
      ctx.beginPath()
      ctx.arc(47, 16, 10, 0, Math.PI * 2)
      ctx.fill()
      ctx.fillStyle = '#0b0f12'
      ctx.font = 'bold 12px sans-serif'
      ctx.textAlign = 'center'
      ctx.textBaseline = 'middle'
      ctx.fillText(String(Math.min(unread, 9)), 47, 16)
    }
    link.href = canvas.toDataURL('image/png')
  } catch {
    /* the favicon badge is decorative */
  }
}
