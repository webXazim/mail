import { apiFetch } from '../lib/api'

export type CapabilityState = 'server' | 'partial' | 'client_only' | 'planned'

export type Capability = {
  key: string
  state: CapabilityState
  authority: string
}

export type ApiMeta = {
  product: string
  api_version: string
  contract_version: number
  mail_backend: string
  capabilities: Capability[]
}

let cachedMeta: ApiMeta | null = null
let inFlight: Promise<ApiMeta> | null = null

export const capabilitiesApi = {
  async load({ force = false }: { force?: boolean } = {}): Promise<ApiMeta> {
    if (!force && cachedMeta) return cachedMeta
    if (!force && inFlight) return inFlight

    inFlight = apiFetch<ApiMeta>('/api/meta', {}, { retry: false })
      .then((meta) => {
        if (!Number.isInteger(meta.contract_version) || meta.contract_version < 1) {
          throw new Error('Unsupported backend contract')
        }
        cachedMeta = meta
        return meta
      })
      .finally(() => {
        inFlight = null
      })

    return inFlight
  },

  get(key: string): Capability | null {
    return cachedMeta?.capabilities.find((item) => item.key === key) ?? null
  },

  isServerBacked(key: string): boolean {
    const state = this.get(key)?.state
    return state === 'server' || state === 'partial'
  },

  reset() {
    cachedMeta = null
    inFlight = null
  },
}
