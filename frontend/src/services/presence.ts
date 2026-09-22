import { authApi } from './auth'

export type Presence = 'online' | 'away' | 'offline'

const demoSeeded: Record<string, Presence> = {
  'priya@crescentsphere.com': 'online',
  'alex.chen@crescentsphere.com': 'away',
}

export const presenceOf = (email: string): Presence =>
  authApi.isDemo() ? demoSeeded[email.toLowerCase()] ?? 'offline' : 'offline'

export const presenceLabel: Record<Presence, string> = {
  online: 'Online',
  away: 'Away',
  offline: 'Offline',
}

/** Presence is not yet a production capability. Never fabricate it in a live session. */
export const onlineCount = (): number =>
  authApi.isDemo() ? Object.values(demoSeeded).filter((status) => status !== 'offline').length : 0
