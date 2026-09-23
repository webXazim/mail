import { beforeEach, describe, expect, it, vi } from 'vitest'
import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { MemoryRouter } from 'react-router-dom'
import { CreateAccountPage } from './CreateAccountPage'
import { authApi } from '../services/auth'

const navigateMock = vi.fn()

vi.mock('react-router-dom', async (importOriginal) => {
  const actual = await importOriginal<typeof import('react-router-dom')>()
  return { ...actual, useNavigate: () => navigateMock }
})

vi.mock('../services/auth', () => ({ authApi: { register: vi.fn() } }))

describe('CreateAccountPage', () => {
  beforeEach(() => {
    navigateMock.mockReset()
    vi.mocked(authApi.register).mockReset()
  })

  it('opens verification with the registered email after a successful signup', async () => {
    const user = userEvent.setup()
    vi.mocked(authApi.register).mockResolvedValue({ ok: true, message: 'Check your inbox' })
    render(<MemoryRouter><CreateAccountPage /></MemoryRouter>)

    await user.type(screen.getByLabelText('Full name'), 'Alex Morgan')
    await user.type(screen.getByLabelText('Email address'), 'Alex@Example.com')
    await user.type(screen.getByLabelText('Password', { exact: true }), 'Long-Test-Password!1')
    await user.type(screen.getByLabelText('Confirm password'), 'Long-Test-Password!1')
    await user.click(screen.getByRole('button', { name: /create account/i }))

    await vi.waitFor(() => expect(navigateMock).toHaveBeenCalledWith('/verify-email', {
      state: { email: 'alex@example.com' },
    }))
  })
})
