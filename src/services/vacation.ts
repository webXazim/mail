export type VacationSettings = {
  enabled: boolean
  subject: string
  message: string
  onlyContacts: boolean
  startsAt: string
  endsAt: string
}

export const defaultVacation: VacationSettings = {
  enabled: false,
  subject: 'Out of office',
  message: "Thanks for your email. I'm currently out of office and will get back to you as soon as I can.",
  onlyContacts: true,
  startsAt: '',
  endsAt: '',
}

const vacationKey = 'harbor-mail:vacation'

export const vacationApi = {
  load(): VacationSettings {
    try {
      return { ...defaultVacation, ...JSON.parse(localStorage.getItem(vacationKey) || '{}') }
    } catch {
      return defaultVacation
    }
  },
  save(next: VacationSettings) {
    localStorage.setItem(vacationKey, JSON.stringify(next))
    return next
  },
}