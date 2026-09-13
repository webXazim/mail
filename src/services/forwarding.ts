export type ForwardingSettings = { enabled: boolean; address: string; keepCopy: boolean }

export const defaultForwarding: ForwardingSettings = { enabled: false, address: '', keepCopy: true }

const forwardingKey = 'harbor-mail:forwarding'

export const forwardingApi = {
  load(): ForwardingSettings {
    try {
      return { ...defaultForwarding, ...JSON.parse(localStorage.getItem(forwardingKey) || '{}') }
    } catch {
      return defaultForwarding
    }
  },
  save(next: ForwardingSettings) {
    localStorage.setItem(forwardingKey, JSON.stringify(next))
    return next
  },
}
