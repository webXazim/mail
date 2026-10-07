import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { tokenStore } from '../lib/api'
import { settingsApi } from './settings'

const jsonResponse = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } })

describe('default appearance', () => {
  afterEach(() => { localStorage.removeItem('cs-mail:settings') })

  it('starts in light mode when no theme has been saved', () => {
    localStorage.removeItem('cs-mail:settings')
    expect(settingsApi.load().theme).toBe('light')
  })

  it('preserves a saved dark theme preference', () => {
    localStorage.setItem('cs-mail:settings', JSON.stringify({ theme: 'dark' }))
    expect(settingsApi.load().theme).toBe('dark')
  })
})

describe('settingsApi.deleteMyAccount — WS5.4 customer self-service erasure', () => {
  beforeEach(() => {
    tokenStore.clear()
    tokenStore.setAccess('tok-customer-1')
    vi.stubGlobal('fetch', vi.fn())
  })

  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('re-confirms the password, POSTs /api/account/delete and resolves on ok:true', async () => {
    const fetchMock = vi.mocked(fetch)
    fetchMock.mockResolvedValue(jsonResponse({ ok: true }))

    await expect(settingsApi.deleteMyAccount('Str0ng-Pass!23')).resolves.toEqual({ ok: true })

    const [url, init] = fetchMock.mock.calls[0]
    expect(url).toContain('/api/account/delete')
    expect(init?.method).toBe('POST')
    const headers = new Headers(init?.headers)
    expect(headers.get('Content-Type')).toContain('application/json')
    expect(headers.has('Authorization')).toBe(true)
    expect(JSON.parse(String(init?.body))).toEqual({ password: 'Str0ng-Pass!23' })
  })

  it('propagates a 401 (wrong password) as an ApiError instead of erasing', async () => {
    vi.mocked(fetch).mockResolvedValue(
      jsonResponse(
        { error: 'unauthorized', message: 'Password is incorrect' },
        401,
      ),
    )

    await expect(settingsApi.deleteMyAccount('Definately-Wrong!1')).rejects.toMatchObject({
      status: 401,
      code: 'unauthorized',
    })
  })
})
