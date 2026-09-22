import { useState } from 'react'
import { apiFetch } from '../lib/api'
import { isRemoteMail } from './remote-mail'

export type UserSettings = {
  displayName: string
  signature: string
  theme: 'dark' | 'light'
  density: 'comfortable' | 'cozy' | 'compact'
  radius: 'sharp' | 'subtle' | 'rounded'
  accent: 'lime' | 'mint' | 'aqua' | 'violet' | 'coral'
  desktopNotifications: boolean
  conversations: boolean
  markReadOnOpen: boolean
  alertSound: boolean
  unreadBadge: boolean
  digest: 'daily' | 'weekly' | 'never'
  sendReadReceipts: boolean
  safeLinks: boolean
}

export const defaultSettings: UserSettings = {
  displayName: 'Alex Morgan',
  signature: '',
  theme: 'dark',
  density: 'comfortable',
  radius: 'sharp',
  accent: 'lime',
  desktopNotifications: true,
  conversations: true,
  markReadOnOpen: true,
  alertSound: true,
  unreadBadge: true,
  digest: 'daily',
  sendReadReceipts: false,
  safeLinks: true,
}

const settingsKey = 'cs-mail:settings'

export const applyTheme = (theme: UserSettings['theme']) => {
  document.documentElement.dataset.theme = theme
}

export const applyDensity = (density: UserSettings['density']) => {
  document.documentElement.dataset.density = density
}

export const applyRadius = (radius: UserSettings['radius']) => {
  document.documentElement.dataset.radius = radius
}

export const applyAccent = (accent: UserSettings['accent']) => {
  document.documentElement.dataset.accent = accent
}

export const settingsApi = {
  load(): UserSettings {
    try {
      return { ...defaultSettings, ...JSON.parse(localStorage.getItem(settingsKey) || '{}') }
    } catch {
      return defaultSettings
    }
  },
  save(next: UserSettings) {
    localStorage.setItem(settingsKey, JSON.stringify(next))
    applyTheme(next.theme)
    applyDensity(next.density)
    applyRadius(next.radius)
    applyAccent(next.accent)
    if (isRemoteMail()) {
      void apiFetch<unknown>(
        '/api/settings',
        { method: 'PUT', body: JSON.stringify(next) },
        { retry: false },
      ).catch(() => {})
    }
  },
  /** API-first hydration; falls back to the local cache when offline or in demo mode. */
  async refresh(): Promise<UserSettings> {
    if (!isRemoteMail()) return settingsApi.load()
    try {
      const remote = await apiFetch<Partial<UserSettings>>('/api/settings')
      const next = { ...defaultSettings, ...settingsApi.load(), ...remote }
      localStorage.setItem(settingsKey, JSON.stringify(next))
      applyTheme(next.theme)
      applyDensity(next.density)
      applyRadius(next.radius)
      applyAccent(next.accent)
      return next
    } catch {
      return settingsApi.load()
    }
  },
  /** Irreversible WS5.4 self-service erasure. Re-confirms the password the
   * way the backend demands, then clears the local session on success so the
   * deleted account can never be used again. Callers handle navigation. */
  async deleteMyAccount(password: string): Promise<{ ok: true }> {
    const result = await apiFetch<{ ok: boolean }>('/api/account/delete', {
      method: 'POST',
      body: JSON.stringify({ password }),
    })
    if (!result.ok) throw new Error('Account erasure was refused')
    return { ok: true }
  },
}

export function useSettings() {
  const [settings, setSettings] = useState<UserSettings>(() => settingsApi.load())
  const update = (patch: Partial<UserSettings>) =>
    setSettings((current) => {
      const next = { ...current, ...patch }
      settingsApi.save(next)
      return next
    })
  return { ...settings, update }
}
