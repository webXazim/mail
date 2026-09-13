export type Presence = 'online' | 'away' | 'offline'

const seeded: Record<string, Presence> = {
  'priya@harbor.co': 'online',
  'alex.chen@harbor.co': 'away',
}

export const presenceOf = (email: string): Presence => seeded[email.toLowerCase()] ?? 'offline'

export const presenceLabel: Record<Presence, string> = {
  online: 'Online',
  away: 'Away',
  offline: 'Offline',
}

export const onlineCount = (): number =>
  Object.values(seeded).filter((status) => status !== 'offline').length
