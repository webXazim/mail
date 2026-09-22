import { authApi } from '../services/auth'
import { capabilitiesApi, type CapabilityState } from '../services/capabilities'

/**
 * Backend migration guard. Local persistence is permitted for explicit demo
 * builds, but authenticated production sessions must never silently mistake a
 * browser-only implementation for server authority.
 */
export function authorityState(feature: string): CapabilityState | 'unknown' {
  if (authApi.isDemo()) return 'client_only'
  return capabilitiesApi.get(feature)?.state ?? 'unknown'
}

export function isAuthoritativeOnServer(feature: string): boolean {
  const state = authorityState(feature)
  return state === 'server' || state === 'partial'
}
