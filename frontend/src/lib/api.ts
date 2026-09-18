/** In-memory access token (the refresh lives in the httpOnly session cookie). */

let accessToken: string | null = null

export const tokenStore = {
  getAccess: () => accessToken,
  /** Store an access token in memory only. Never touch localStorage. */
  setAccess: (token: string) => {
    accessToken = token
  },
  /** Legacy two-arg signature kept so auth.ts keeps compiling untouched. */
  set: (access: string, _refresh?: string) => {
    accessToken = access
  },
  getRefresh: () => null,
  clear: () => {
    accessToken = null
  },
}

/** Minimal session presence used by RequireAuth on hot-reload / navigation. */
export function isSessionActive(): boolean {
  return accessToken !== null
}

export class ApiError extends Error {
  status: number
  code: string

  constructor(status: number, message: string, code = 'api_error') {
    super(message)
    this.status = status
    this.code = code
  }
}

type ApiResponse = {
  status: number
  body: unknown
}

async function request(path: string, options: RequestInit = {}): Promise<ApiResponse> {
  const url = path.startsWith('http')
    ? path
    : path.startsWith('/api/')
      ? path
      : `/api${path.startsWith('/') ? path : `/${path}`}`
  const headers = new Headers(options.headers)
  headers.set('Content-Type', 'application/json')
  const access = tokenStore.getAccess()
  if (access) headers.set('Authorization', `Bearer ${access}`)

  const response = await fetch(url, { ...options, headers, credentials: 'include' })
  let body: unknown = null
  const text = await response.text()
  if (text) {
    try {
      body = JSON.parse(text)
    } catch {
      body = text
    }
  }

  return { status: response.status, body }
}

export async function apiFetch<T = unknown>(
  path: string,
  options: RequestInit = {},
  { retry = true } = {},
): Promise<T> {
  let result = await request(path, options)

  if (result.status === 401 && retry) {
    const refreshed = await refreshSession()
    if (refreshed) {
      result = await request(path, options)
    }
  }

  if (result.status >= 400) {
    const message =
      result.body && typeof result.body === 'object' && 'message' in result.body
        ? String((result.body as Record<string, unknown>).message)
        : `Request failed (${result.status})`
    const code =
      result.body && typeof result.body === 'object' && 'error' in result.body
        ? String((result.body as Record<string, unknown>).error)
        : 'api_error'
    throw new ApiError(result.status, message, code)
  }

  return result.body as T
}

let refreshInFlight: Promise<boolean> | null = null

export function refreshSession(): Promise<boolean> {
  if (!refreshInFlight) {
    refreshInFlight = refreshImpl().finally(() => {
      refreshInFlight = null
    })
  }
  return refreshInFlight
}

/** Restore a session purely from the httpOnly cookie on boot. */
export function bootstrapSession(): Promise<boolean> {
  return refreshSession()
}

async function refreshImpl(): Promise<boolean> {
  try {
    const { body } = await request('/api/auth/refresh', { method: 'POST' })
    if (body && typeof body === 'object' && 'access' in body) {
      tokenStore.setAccess((body as { access: string }).access)
      return true
    }
    tokenStore.clear()
    return false
  } catch {
    tokenStore.clear()
    return false
  }
}
