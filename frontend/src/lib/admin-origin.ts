/**
 * Platform administration is intentionally served only from the loopback
 * operator endpoint (normally reached through an SSH tunnel). The reverse
 * proxy and Rust AdminUser extractor are the security boundary; this helper
 * keeps the public UI from advertising routes that are guaranteed to 404.
 */
export function isLocalAdminOrigin(): boolean {
  if (typeof window === 'undefined') return false
  const host = window.location.hostname.toLowerCase()
  return host === 'localhost' || host === '127.0.0.1' || host === '::1'
}
