import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { ApiError, apiFetch, tokenStore } from './api'

const jsonResponse = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } })

describe('apiFetch', () => {
  beforeEach(() => {
    tokenStore.clear()
    vi.stubGlobal('fetch', vi.fn())
  })

  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('prepends /api, attaches the bearer token and parses the JSON body', async () => {
    tokenStore.setAccess('tok-123')
    const fetchMock = vi.mocked(fetch)
    fetchMock.mockResolvedValue(jsonResponse({ ok: true }))

    const result = await apiFetch<{ ok: boolean }>('/health')
    expect(result).toEqual({ ok: true })

    const [url, init] = fetchMock.mock.calls[0]
    expect(url).toBe('/api/health')
    expect(init?.headers).toBeInstanceOf(Headers)
    expect(new Headers(init?.headers).get('Authorization')).toBe('Bearer tok-123')
  })

  it('throws an ApiError carrying the server message and code on failure', async () => {
    vi.mocked(fetch).mockResolvedValue(
      jsonResponse({ error: 'forbidden', message: 'Admins only' }, 403),
    )

    await expect(apiFetch('/api/admin/users')).rejects.toMatchObject({
      status: 403,
      code: 'forbidden',
      message: 'Admins only',
    })
  })

  it('retries once through the refresh flow on a 401 and succeeds', async () => {
    const fetchMock = vi.mocked(fetch)
    fetchMock
      .mockResolvedValueOnce(jsonResponse({ error: 'unauthorized', message: 'Expired' }, 401))
      .mockResolvedValueOnce(jsonResponse({ access: 'rotated-access' }))
      .mockResolvedValueOnce(jsonResponse({ ok: true }))

    const result = await apiFetch<{ ok: boolean }>('/me')
    expect(result).toEqual({ ok: true })
    expect(tokenStore.getAccess()).toBe('rotated-access')
    expect(fetchMock).toHaveBeenCalledTimes(3)
  })

  it('surfaces a generic message when the failure body has none', async () => {
    vi.mocked(fetch).mockResolvedValue(new Response('oops', { status: 500 }))
    await expect(apiFetch('/x')).rejects.toMatchObject({
      status: 500,
      message: 'Request failed (500)',
    })
  })

  it('apiFetch errors are ApiError instances', async () => {
    vi.mocked(fetch).mockResolvedValue(jsonResponse({ error: 'nope', message: 'Nope' }, 400))
    const error = await apiFetch('/x').catch((caught: unknown) => caught)
    expect(error).toBeInstanceOf(ApiError)
  })
})