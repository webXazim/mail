import { afterEach, describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { discoverCsAccounts, useCsAccounts, type CsService } from './csAccounts'

afterEach(() => vi.unstubAllGlobals())

describe('CS account discovery', () => {
  it.each<CsService>(['connect', 'docs', 'mail', 'mailer'])('checks only other services for %s with cookies and no cache', async current => {
    const fetchMock = vi.fn().mockResolvedValue({ ok: true, json: async () => ({ signed_in: true }) })
    vi.stubGlobal('fetch', fetchMock)
    const accounts = await discoverCsAccounts(current)
    expect(accounts).toHaveLength(3)
    expect(accounts).not.toContain(current)
    for (const [url, options] of fetchMock.mock.calls) {
      expect(new URL(url).hostname).not.toBe(`${current}.crescentsphere.com`)
      expect(options).toMatchObject({ credentials: 'include', cache: 'no-store' })
    }
  })

  it('does not mistake signed-out, failed or malformed responses for an account', async () => {
    vi.stubGlobal('fetch', vi.fn()
      .mockResolvedValueOnce({ ok: true, json: async () => ({ signed_in: false }) })
      .mockRejectedValueOnce(new Error('Unavailable'))
      .mockResolvedValueOnce({ ok: true, json: async () => ({ signed_in: 'true' }) }))
    expect(await discoverCsAccounts('mail')).toEqual([])
  })

  it('shows nothing until a session is found and hides the option after logout is detected', async () => {
    let signedIn = true
    vi.stubGlobal('fetch', vi.fn().mockImplementation(async () => ({ ok: true, json: async () => ({ signed_in: signedIn }) })))
    function Options() {
      const accounts = useCsAccounts('mail', true)
      return accounts.length ? <button>Sign in with CS account</button> : null
    }
    render(<Options />)
    expect(screen.queryByRole('button')).not.toBeInTheDocument()
    expect(await screen.findByRole('button')).toHaveTextContent('Sign in with CS account')
    signedIn = false
    fireEvent.focus(window)
    await waitFor(() => expect(screen.queryByRole('button')).not.toBeInTheDocument())
  })

  it('does not probe accounts when federation is disabled', async () => {
    const fetchMock = vi.fn()
    vi.stubGlobal('fetch', fetchMock)
    function Disabled() {
      const accounts = useCsAccounts('mail', false)
      return <span>{accounts.length}</span>
    }
    render(<Disabled />)
    await waitFor(() => expect(screen.getByText('0')).toBeInTheDocument())
    expect(fetchMock).not.toHaveBeenCalled()
  })
})
