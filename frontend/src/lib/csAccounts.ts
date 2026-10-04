import { useEffect, useState } from 'react'

export type CsService = 'connect' | 'docs' | 'mail' | 'mailer'
const endpoints: Record<CsService, string> = {
  connect: 'https://connect.crescentsphere.com/api/v1/accounts/federation/session-account/?presence=1',
  docs: 'https://docs.crescentsphere.com/api/v1/auth/federation/session-account/?presence=1',
  mail: 'https://mail.crescentsphere.com/api/auth/federation/session-account',
  mailer: 'https://mailer.crescentsphere.com/api/v1/auth/federation/session-account',
}

export async function discoverCsAccounts(current: CsService, signal?: AbortSignal): Promise<CsService[]> {
  const results = await Promise.all((Object.keys(endpoints) as CsService[]).filter(service => service !== current).map(async service => {
    const controller = new AbortController()
    const abort = () => controller.abort()
    signal?.addEventListener('abort', abort, { once: true })
    if (signal?.aborted) controller.abort()
    const timeout = setTimeout(abort, 4000)
    try {
      const response = await fetch(endpoints[service], { credentials: 'include', cache: 'no-store', signal: controller.signal })
      if (!response.ok) return null
      const payload: unknown = await response.json()
      return payload && typeof payload === 'object' && (payload as { signed_in?: unknown }).signed_in === true ? service : null
    } catch { return null }
    finally { clearTimeout(timeout); signal?.removeEventListener('abort', abort) }
  }))
  return results.filter((service): service is CsService => service !== null)
}

export function useCsAccounts(current: CsService, enabled: boolean) {
  const [accounts, setAccounts] = useState<CsService[]>([])
  useEffect(() => {
    let active = true, controller: AbortController | undefined
    const refresh = () => {
      controller?.abort()
      const request = new AbortController()
      controller = request
      void (enabled ? discoverCsAccounts(current, request.signal) : Promise.resolve([] as CsService[]))
        .then(found => { if (active && !request.signal.aborted) setAccounts(found) })
    }
    const visible = () => { if (document.visibilityState === 'visible') refresh() }
    refresh()
    window.addEventListener('focus', refresh)
    document.addEventListener('visibilitychange', visible)
    const timer = window.setInterval(visible, 30000)
    return () => { active = false; controller?.abort(); clearInterval(timer); window.removeEventListener('focus', refresh); document.removeEventListener('visibilitychange', visible) }
  }, [current, enabled])
  return enabled ? accounts : []
}
