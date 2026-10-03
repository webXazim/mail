import { StrictMode } from 'react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { render, screen, waitFor } from '@testing-library/react'
import { MemoryRouter } from 'react-router-dom'
import { ConnectCallbackPage } from './ConnectCallbackPage'
import { authApi } from '../services/auth'

const navigateMock = vi.fn()
vi.mock('react-router-dom', async (importOriginal) => {
  const actual = await importOriginal<typeof import('react-router-dom')>()
  return { ...actual, useNavigate: () => navigateMock }
})
vi.mock('../services/auth', () => ({ authApi: { completeConnect: vi.fn() } }))

function validCallback() {
  sessionStorage.setItem('cs-mail:connect-signin', JSON.stringify({
    state: 'expected-state', verifier: 'pkce-verifier', createdAt: Date.now(), next: '',
  }))
  history.replaceState(null, '', '/auth/connect/callback?code=authorization-code&state=expected-state')
}
function mount() {
  return render(<StrictMode><MemoryRouter><ConnectCallbackPage /></MemoryRouter></StrictMode>)
}

describe('Connect callback', () => {
  beforeEach(() => {
    sessionStorage.clear()
    history.replaceState(null, '', '/auth/connect/callback')
    vi.mocked(authApi.completeConnect).mockReset()
    navigateMock.mockReset()
  })

  it('shows invalid callback errors without exchanging a code', async () => {
    mount()
    expect(await screen.findByRole('alert')).toHaveTextContent('cancelled or expired')
    expect(authApi.completeConnect).not.toHaveBeenCalled()
    expect(navigateMock).not.toHaveBeenCalled()
  })

  it('exchanges a valid callback once under StrictMode before navigating', async () => {
    validCallback()
    vi.mocked(authApi.completeConnect).mockResolvedValue({} as Awaited<ReturnType<typeof authApi.completeConnect>>)
    mount()
    await waitFor(() => expect(navigateMock).toHaveBeenCalledWith('/mail/business', { replace: true }))
    expect(authApi.completeConnect).toHaveBeenCalledTimes(1)
    expect(authApi.completeConnect).toHaveBeenCalledWith('authorization-code', 'pkce-verifier')
    expect(sessionStorage.getItem('cs-mail:connect-signin')).toBeNull()
  })

  it('shows an exchange failure and keeps the user on the callback', async () => {
    validCallback()
    vi.mocked(authApi.completeConnect).mockRejectedValue(new Error('Account verification failed'))
    mount()
    expect(await screen.findByRole('alert')).toHaveTextContent('Account verification failed')
    expect(navigateMock).not.toHaveBeenCalled()
  })
})
