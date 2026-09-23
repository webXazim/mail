import { beforeEach, describe, expect, it, vi } from 'vitest'
import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { MemoryRouter } from 'react-router-dom'
import { LoginPage } from './LoginPage'
import { authApi } from '../services/auth'
import { auditApi } from '../services/audit'

const navigateMock = vi.fn()

vi.mock('react-router-dom', async (importOriginal) => {
  const actual = await importOriginal<typeof import('react-router-dom')>()
  return { ...actual, useNavigate: () => navigateMock }
})

vi.mock('../services/auth', () => ({
  authApi: { login: vi.fn(), verifyTwoFactor: vi.fn() },
}))

vi.mock('../services/audit', () => ({
  auditApi: { add: vi.fn() },
}))

const mount = () => render(<MemoryRouter><LoginPage /></MemoryRouter>)

describe('LoginPage', () => {
  beforeEach(() => {
    navigateMock.mockClear()
    vi.mocked(authApi.login).mockReset()
    vi.mocked(authApi.verifyTwoFactor).mockReset()
    vi.mocked(auditApi.add).mockReset()
  })

  it('signs in, records the audit event and navigates to the mailbox', async () => {
    const user = userEvent.setup()
    vi.mocked(authApi.login).mockResolvedValue({
      access: 'tok',
      refresh: 'ref',
      user: { id: 'u1', email: 'alex@cs-mail.test', display_name: 'Alex', role: 'admin' },
    })
    mount()

    await user.type(screen.getByLabelText('Email address'), 'alex@cs-mail.test')
    await user.type(screen.getByLabelText('Password'), 'Strong-Pass!1')
    await user.click(screen.getByRole('button', { name: /sign in/i }))

    expect(authApi.login).toHaveBeenCalledWith('alex@cs-mail.test', 'Strong-Pass!1')
    await vi.waitFor(() => expect(navigateMock).toHaveBeenCalledWith('/mail/inbox', { replace: true }))
    expect(auditApi.add).toHaveBeenCalled()
  })

  it('requires and verifies the second factor before navigating', async () => {
    const user = userEvent.setup()
    vi.mocked(authApi.login).mockResolvedValue({
      two_factor_required: true,
      challenge_token: 'challenge-1',
      expires_in: 300,
    })
    vi.mocked(authApi.verifyTwoFactor).mockResolvedValue({
      access: 'tok',
      user: { id: 'u1', email: 'alex@cs-mail.test', display_name: 'Alex', role: 'admin' },
    })
    mount()

    await user.type(screen.getByLabelText('Email address'), 'alex@cs-mail.test')
    await user.type(screen.getByLabelText('Password'), 'Strong-Pass!1')
    await user.click(screen.getByRole('button', { name: /sign in/i }))

    expect(await screen.findByLabelText('Authentication code')).toBeInTheDocument()
    expect(navigateMock).not.toHaveBeenCalled()
    await user.type(screen.getByLabelText('Authentication code'), '123456')
    await user.click(screen.getByRole('button', { name: /verify and sign in/i }))

    expect(authApi.verifyTwoFactor).toHaveBeenCalledWith('challenge-1', '123456')
    await vi.waitFor(() => expect(navigateMock).toHaveBeenCalledWith('/mail/inbox', { replace: true }))
  })

  it('surfaces the login error instead of navigating', async () => {
    const user = userEvent.setup()
    vi.mocked(authApi.login).mockRejectedValue(new Error('Invalid email or password'))
    mount()

    await user.type(screen.getByLabelText('Email address'), 'alex@cs-mail.test')
    await user.type(screen.getByLabelText('Password'), 'Strong-Pass!1')
    await user.click(screen.getByRole('button', { name: /sign in/i }))

    const alert = await screen.findByRole('alert')
    expect(alert).toHaveTextContent('Invalid email or password')
    expect(navigateMock).not.toHaveBeenCalled()
  })

  it('toggles password visibility from the reveal button', async () => {
    const user = userEvent.setup()
    mount()

    const input = screen.getByLabelText('Password')
    expect(input).toHaveAttribute('type', 'password')
    await user.click(screen.getByRole('button', { name: 'Show password' }))
    expect(input).toHaveAttribute('type', 'text')
    await user.click(screen.getByRole('button', { name: 'Hide password' }))
    expect(input).toHaveAttribute('type', 'password')
  })
})
