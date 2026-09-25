import { useCallback, useEffect, useId, useRef, useState } from 'react'
import { sanitizeHtml } from '../lib/sanitize'

type MailBodyFrameProps = {
  /** raw (unsanitized) HTML from the mailbox bridge — never trusted */
  html: string
  /** mail subject, used only as the frame's accessible name */
  subject: string
  /** Remote images are opt-in because they can act as tracking beacons. */
  allowRemoteImages?: boolean
}

/**
 * WS5.2 render path for HTML mail — two independent layers:
 *  1. DOMPurify allow-list sanitizer (all <script>/on* handlers/javascript:
 *     URIs stripped; even a bypass leaves nothing executable).
 *  2. <iframe sandbox> WITHOUT allow-scripts, forms, navigation, or popups.
 *     Same-origin access is limited to the parent measuring the sanitized
 *     srcdoc so the frame fits its content instead of reserving empty space.
 */
export function MailBodyFrame({ html, subject, allowRemoteImages = false }: MailBodyFrameProps) {
  const frameId = useId().replace(/[:]/g, '')
  const frameRef = useRef<HTMLIFrameElement>(null)
  const observerRef = useRef<ResizeObserver | null>(null)
  const [height, setHeight] = useState(48)
  const safeHtml = sanitizeHtml(html)
  const imagePolicy = allowRemoteImages ? 'data: https: http:' : 'data:'
  const csp = `default-src 'none'; img-src ${imagePolicy}; style-src 'unsafe-inline'; font-src 'none'; connect-src 'none'; frame-src 'none';`
  const lightTheme = document.documentElement.dataset.theme === 'light'
  const frameBackground = lightTheme ? '#fbfcf8' : '#111817'
  const frameColor = lightTheme ? '#4b5650' : '#d9e0dc'

  const fitContent = useCallback(() => {
    const document = frameRef.current?.contentDocument
    if (!document) return
    const next = Math.max(
      48,
      document.body?.scrollHeight ?? 0,
      document.documentElement?.scrollHeight ?? 0,
    )
    setHeight(next)
  }, [])

  const onLoad = useCallback(() => {
    observerRef.current?.disconnect()
    fitContent()
    const body = frameRef.current?.contentDocument?.body
    if (body && typeof ResizeObserver !== 'undefined') {
      observerRef.current = new ResizeObserver(fitContent)
      observerRef.current.observe(body)
    }
  }, [fitContent])

  useEffect(() => () => observerRef.current?.disconnect(), [])

  return (
    <iframe
      ref={frameRef}
      id={`mail-body-frame-${frameId}`}
      className="mail-body-frame"
      title={`Message body — ${subject}`}
      sandbox="allow-same-origin"
      referrerPolicy="no-referrer"
      scrolling="no"
      onLoad={onLoad}
      srcDoc={`<!doctype html><html><head><meta charset="utf-8" /><meta http-equiv="Content-Security-Policy" content="${csp}" /><style>html,body{margin:0;background:${frameBackground}!important}body{color:${frameColor};font-family:system-ui,sans-serif;font-size:13.5px;line-height:1.7;padding:12px 0;overflow-wrap:anywhere}img{max-width:100%;height:auto}</style></head><body>${safeHtml}</body></html>`}
      style={{
        width: '100%',
        border: 'none',
        height,
        overflow: 'hidden',
        background: frameBackground,
      }}
    />
  )
}
