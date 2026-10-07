import { afterEach, expect, it, vi } from 'vitest'
import mailLogo from '../assets/logos/cs-mail.svg'
import { applyFaviconBadge } from './favicon'

afterEach(() => {
  applyFaviconBadge(0)
  document.head.querySelector('link[rel="icon"]')?.remove()
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

it('uses the Mail logo for unread badges and restores it when unread clears', () => {
  const link = document.createElement('link')
  link.rel = 'icon'
  document.head.append(link)
  let loadedImage: HTMLImageElement
  vi.stubGlobal('Image', class {
    complete = true
    naturalWidth = 64
    src = ''
    onload = null
    constructor() { loadedImage = this as unknown as HTMLImageElement }
  })
  const drawImage = vi.fn()
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue({
    drawImage, beginPath: vi.fn(), arc: vi.fn(), fill: vi.fn(), fillText: vi.fn(),
  } as unknown as CanvasRenderingContext2D)
  vi.spyOn(HTMLCanvasElement.prototype, 'toDataURL').mockReturnValue('data:image/png;base64,badge')

  applyFaviconBadge(3)
  expect(loadedImage!.src).toBe(mailLogo)
  expect(drawImage).toHaveBeenCalledWith(loadedImage!, 0, 0, 64, 64)
  expect(link.type).toBe('image/png')
  expect(link.getAttribute('href')).toBe('data:image/png;base64,badge')

  applyFaviconBadge(0)
  expect(link.type).toBe('image/svg+xml')
  expect(link.getAttribute('href')).toBe(mailLogo)
})
