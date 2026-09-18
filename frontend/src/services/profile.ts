import { useSyncExternalStore } from 'react'
import { apiFetch } from '../lib/api'
import { primaryAccount, setPrimaryIdentity } from './accounts'
import { isDemoAllowed } from './auth'
import { isRemoteMail } from './remote-mail'
import { defaultSettings, settingsApi } from './settings'

export type Profile = {
  id: string
  email: string
  display_name: string
  role: string
  onboarded: boolean
  storage: { used_bytes: number; total_bytes: number; pct: number }
}

const profileKey = 'harbor-mail:profile'

export function displayNameOf(profile: Profile): string {
  return profile.display_name.trim() || profile.email.split('@')[0]
}

function readCache(): Profile | null {
  try {
    const raw = localStorage.getItem(profileKey)
    return raw ? (JSON.parse(raw) as Profile) : null
  } catch {
    return null
  }
}

let current: Profile | null = readCache()
const listeners = new Set<() => void>()

function setCurrent(profile: Profile | null) {
  current = profile
  listeners.forEach((listener) => listener())
}

export function subscribeProfile(listener: () => void) {
  listeners.add(listener)
  return () => {
    listeners.delete(listener)
  }
}

export function currentProfile(): Profile | null {
  return current
}

/** Reactive profile; null until hydrated (or in demo mode). */
export function useProfile(): Profile | null {
  return useSyncExternalStore(subscribeProfile, currentProfile, () => null)
}

/**
 * The signed-in user's role. Demo builds resolve to `admin` so local
 * development keeps the admin surfaces; real sessions read the profile.
 */
export function useRole(): string | null {
  const profile = useProfile()
  if (profile?.role) return profile.role
  if (isDemoAllowed() && localStorage.getItem('harbor-mail:demo') === 'true') return 'admin'
  return null
}

/**
 * Seed local identity from the real profile. The account switcher, composer
 * From, and settings name all read the signed-in user rather than the demo
 * fixture once this runs.
 */
function applyIdentity(profile: Profile) {
  const name = displayNameOf(profile)
  setPrimaryIdentity({ name, email: profile.email })
  const current = settingsApi.load()
  if (current.displayName === defaultSettings.displayName) {
    settingsApi.save({ ...current, displayName: name })
  }
}

export const profileApi = {
  cached: readCache,
  /** API-first hydration; returns the cache when offline or in demo mode. */
  async refresh(): Promise<Profile | null> {
    if (!isRemoteMail()) return null
    try {
      const profile = await apiFetch<Profile>('/api/profile')
      localStorage.setItem(profileKey, JSON.stringify(profile))
      setCurrent(profile)
      applyIdentity(profile)
      return profile
    } catch {
      const cached = readCache()
      setCurrent(cached)
      return cached
    }
  },
  async update(patch: { display_name?: string; onboarded?: boolean }): Promise<Profile | null> {
    if (!isRemoteMail()) return null
    const profile = await apiFetch<Profile>('/api/profile', {
      method: 'PUT',
      body: JSON.stringify(patch),
    })
    localStorage.setItem(profileKey, JSON.stringify(profile))
    setCurrent(profile)
    applyIdentity(profile)
    return profile
  },
}

/** Best-known identity for the signed-in user (profile, else cached account). */
export function localIdentity(): { name: string; email: string } {
  if (current) return { name: displayNameOf(current), email: current.email }
  const account = primaryAccount()
  return { name: account.name, email: account.email }
}
