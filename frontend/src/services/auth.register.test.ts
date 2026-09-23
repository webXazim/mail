import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { authApi } from './auth'
import { isSessionActive, tokenStore } from '../lib/api'

describe('registration awaiting email verification', () => {
  beforeEach(() => {
    tokenStore.clear()
    vi.stubGlobal('fetch', vi.fn())
  })

  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('keeps the visitor signed out when registration returns no access token', async () => {
    vi.mocked(fetch).mockResolvedValue(new Response(
      JSON.stringify({ ok: true, message: 'Check your inbox to verify your email.' }),
      { status: 200, headers: { 'Content-Type': 'application/json' } },
    ))

    const result = await authApi.register('Alex', 'alex@example.com', 'Long-Test-Password!1')

    expect(result).toMatchObject({ ok: true })
    expect(tokenStore.getAccess()).toBeNull()
    expect(isSessionActive()).toBe(false)
  })
})
