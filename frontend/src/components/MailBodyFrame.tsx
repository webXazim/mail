import { useId } from 'react'
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
 *  2. <iframe sandbox> WITHOUT allow-scripts / allow-same-origin, so anything
 *     that survives sanitization still cannot execute or read the app origin.
 */
export function MailBodyFrame({ html, subject, allowRemoteImages = false }: MailBodyFrameProps) {
  const frameId = useId().replace(/[:]/g, '')
  const safeHtml = sanitizeHtml(html)
  const imagePolicy = allowRemoteImages ? "data: https: http:" : "data:"
  const csp = `default-src 'none'; img-src ${imagePolicy}; style-src 'unsafe-inline'; font-src 'none'; connect-src 'none'; frame-src 'none';`

  return (
    <iframe
      id={`mail-body-frame-${frameId}`}
      className="mail-body-frame"
      title={`Message body — ${subject}`}
      sandbox=""
      referrerPolicy="no-referrer"
      srcDoc={`<!doctype html><html><head><meta charset="utf-8" /><meta http-equiv="Content-Security-Policy" content="${csp}" /><style>body{font-family:system-ui,sans-serif;line-height:1.5;padding:1rem;word-wrap:break-word}img{max-width:100%}</style></head><body>${safeHtml}</body></html>`}
      style={{ width: '100%', border: 'none', height: 420, overflow: 'auto' }}
    />
  )
}