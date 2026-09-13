import { useState } from 'react'

export type UserSettings = {
  displayName: string
  signature: string
  theme: 'dark' | 'light'
  density: 'compact' | 'cozy' | 'comfortable'
  desktopNotifications: boolean
  conversations: boolean
  markReadOnOpen: boolean
  alertSound: boolean
  unreadBadge: boolean
  digest: 'daily' | 'weekly' | 'never'
  sendReadReceipts: boolean
  twoFactor: boolean
  safeLinks: boolean
}

export const defaultSettings: UserSettings = {
  displayName: 'Alex Morgan',
  signature: '',
  theme: 'dark',
  density: 'comfortable',
  desktopNotifications: true,
  conversations: true,
  markReadOnOpen: true,
  alertSound: true,
  unreadBadge: true,
  digest: 'daily',
  sendReadReceipts: false,
  twoFactor: true,
  safeLinks: true,
}

const settingsKey = 'harbor-mail:settings'

export const applyTheme = (theme: UserSettings['theme']) => {
  document.documentElement.dataset.theme = theme
}

export const applyDensity = (density: UserSettings['density']) => {
  document.documentElement.dataset.density = density
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
