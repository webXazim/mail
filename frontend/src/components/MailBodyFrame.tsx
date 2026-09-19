import { useId } from 'react'
import { sanitizeHtml } from '../lib/sanitize'

type MailBodyFrameProps = {
  /** raw (unsanitized) HTML from the mailbox bridge — never trusted */
  html: string
  /** mail subject, used only as the frame's accessible name */
  subject: string
}

/**
 * WS5.2 render path for HTML mail — two independent layers:
 *  1. DOMPurify allow-list sanitizer (all <script>/on* handlers/javascript:
 *     URIs stripped; even a bypass leaves nothing executable).
 *  2. <iframe sandbox> WITHOUT allow-scripts / allow-same-origin, so anything
 *     that survives sanitization still cannot execute or read the app origin.
 */
export function MailBodyFrame({ html, subject }: MailBodyFrameProps) {
  const frameId = useId().replace(/[:]/g, '')
  const safeHtml = sanitizeHtml(html)

  return (
    <iframe
      id={`mail-body-frame-${frameId}`}
      className="mail-body-frame"
      title={`Message body — ${subject}`}
      sandbox=""
      srcDoc={`<!doctype html><html><head><meta charset="utf-8" /><style>body{font-family:system-ui,sans-serif;line-height:1.5;padding:1rem;word-wrap:break-word}img{max-width:100%}</style></head><body>${safeHtml}</body></html>`}
      style={{ width: '100%', border: 'none', height: 420, overflow: 'auto' }}
    />
  )
}