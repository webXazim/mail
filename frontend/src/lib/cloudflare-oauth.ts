const STORAGE_KEY = 'cs-mail-cloudflare-oauth'

export type PendingCloudflareOAuth = {
  state: string
  verifier: string
  organizationId: string
  domainId: string
  mode: 'setup' | 'mail'
  startedAt: number
}

function base64url(bytes: Uint8Array): string {
  return btoa(String.fromCharCode(...bytes)).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '')
}

function randomValue(): string {
  return base64url(crypto.getRandomValues(new Uint8Array(32)))
}

export async function beginCloudflareOAuth(
  config: { client_id: string; redirect_uri: string; authorization_url: string },
  organizationId: string,
  domainId: string,
  mode: PendingCloudflareOAuth['mode'],
): Promise<void> {
  const verifier = randomValue()
  const state = randomValue()
  const digest = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(verifier))
  const challenge = base64url(new Uint8Array(digest))
  const pending: PendingCloudflareOAuth = { state, verifier, organizationId, domainId, mode, startedAt: Date.now() }
  sessionStorage.setItem(STORAGE_KEY, JSON.stringify(pending))
  const url = new URL(config.authorization_url)
  url.searchParams.set('response_type', 'code')
  url.searchParams.set('client_id', config.client_id)
  url.searchParams.set('redirect_uri', config.redirect_uri)
  url.searchParams.set('scope', 'zone.read dns.write')
  url.searchParams.set('code_challenge', challenge)
  url.searchParams.set('code_challenge_method', 'S256')
  url.searchParams.set('state', state)
  window.location.assign(url.toString())
}

export function readPendingCloudflareOAuth(): PendingCloudflareOAuth | null {
  try {
    const value = sessionStorage.getItem(STORAGE_KEY)
    if (!value) return null
    const pending = JSON.parse(value) as PendingCloudflareOAuth
    if (!pending.state || !pending.verifier || !pending.organizationId || !pending.domainId
      || !['setup', 'mail'].includes(pending.mode) || !Number.isFinite(pending.startedAt)
      || pending.startedAt > Date.now() || Date.now() - pending.startedAt > 10 * 60_000) return null
    return pending
  } catch {
    return null
  }
}

export function clearPendingCloudflareOAuth(): void {
  sessionStorage.removeItem(STORAGE_KEY)
}
