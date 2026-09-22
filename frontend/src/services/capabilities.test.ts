import { beforeEach, describe, expect, it, vi } from 'vitest'
import { capabilitiesApi } from './capabilities'

const fetchMock = vi.fn()
vi.stubGlobal('fetch', fetchMock)

describe('capabilitiesApi', () => {
  beforeEach(() => {
    capabilitiesApi.reset()
    fetchMock.mockReset()
  })

  it('loads and caches the backend contract', async () => {
    fetchMock.mockResolvedValueOnce(
      new Response(
        JSON.stringify({
          product: 'CS Mail',
          api_version: '0.1.0',
          contract_version: 1,
          mail_backend: 'self_hosted',
          capabilities: [{ key: 'mailbox', state: 'server', authority: 'mail_server' }],
        }),
        { status: 200, headers: { 'Content-Type': 'application/json' } },
      ),
    )

    const first = await capabilitiesApi.load()
    const second = await capabilitiesApi.load()

    expect(first).toEqual(second)
    expect(capabilitiesApi.isServerBacked('mailbox')).toBe(true)
    expect(fetchMock).toHaveBeenCalledTimes(1)
  })
})
