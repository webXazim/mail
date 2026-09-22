import { apiFetch } from '../lib/api'
import { isRemoteMail } from './remote-mail'
import type { AutomationSync } from './forwarding'

export type VacationSettings = {
  enabled: boolean
  subject: string
  message: string
  onlyContacts: boolean
  startsAt: string
  endsAt: string
}

export type VacationResponse = { vacation: VacationSettings; sync: AutomationSync }

export const defaultVacation: VacationSettings = {
  enabled: false,
  subject: 'Out of office',
  message:
    "Thanks for your email. I'm currently out of office and will get back to you as soon as I can.",
  onlyContacts: true,
  startsAt: '',
  endsAt: '',
}

const vacationKey = 'cs-mail:vacation'
let remoteCache: VacationSettings = { ...defaultVacation }
let syncCache: AutomationSync | null = null

const normalize = (value: Partial<VacationSettings>): VacationSettings => ({
  ...defaultVacation,
  ...value,
  startsAt: value.startsAt ?? '',
  endsAt: value.endsAt ?? '',
})

const localLoad = (): VacationSettings => {
  try {
    return normalize(JSON.parse(localStorage.getItem(vacationKey) || '{}'))
  } catch {
    return { ...defaultVacation }
  }
}

export const vacationApi = {
  load(): VacationSettings {
    return isRemoteMail() ? { ...remoteCache } : localLoad()
  },
  sync(): AutomationSync | null {
    return syncCache
  },
  async refresh(): Promise<VacationResponse> {
    if (!isRemoteMail()) {
      return {
        vacation: localLoad(),
        sync: {
          status: 'disabled', desiredRevision: 0, appliedRevision: 0, inSync: true, lastError: '',
        },
      }
    }
    const result = await apiFetch<VacationResponse>('/api/mail/vacation')
    remoteCache = normalize(result.vacation)
    syncCache = result.sync
    return { ...result, vacation: { ...remoteCache } }
  },
  async save(next: VacationSettings): Promise<VacationResponse> {
    if (!isRemoteMail()) {
      const local = normalize(next)
      localStorage.setItem(vacationKey, JSON.stringify(local))
      return {
        vacation: local,
        sync: {
          status: 'disabled', desiredRevision: 0, appliedRevision: 0, inSync: true, lastError: '',
        },
      }
    }
    const result = await apiFetch<VacationResponse>('/api/mail/vacation', {
      method: 'PUT',
      body: JSON.stringify({
        ...next,
        startsAt: next.startsAt || null,
        endsAt: next.endsAt || null,
      }),
    })
    remoteCache = normalize(result.vacation)
    syncCache = result.sync
    return { ...result, vacation: { ...remoteCache } }
  },
}
