import { beforeEach, describe, expect, it, vi } from 'vitest'
import { render, screen, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { MemoryRouter } from 'react-router-dom'
import { AdminPage } from './AdminPage'

const mockRemoteAdminApi = vi.hoisted(() => ({
  overview: vi.fn(),
  users: vi.fn(),
  aliases: vi.fn(),
  audit: vi.fn(),
  blockedSenders: vi.fn(),
  createUser: vi.fn(),
  updateUser: vi.fn(),
  deleteUser: vi.fn(),
  createAlias: vi.fn(),
  deleteAlias: vi.fn(),
  blockSender: vi.fn(),
  unblockSender: vi.fn(),
}))

vi.mock('../services/admin', async (importOriginal) => {
  const mod = await importOriginal<typeof import('../services/admin')>()
  return { ...mod, remoteAdminApi: mockRemoteAdminApi }
})

vi.mock('../state/mail/MailContext', () => ({
  useMail: () => ({ reload: vi.fn() }),
}))

const mount = () =>
  render(
    <MemoryRouter>
      <AdminPage />
    </MemoryRouter>,
  )

const clickTab = async (label: RegExp) => {
  const nav = screen.getByRole('navigation', { name: 'Admin sections' })
  await userEvent.click(within(nav).getByRole('button', { name: label }))
}

describe('AdminPage (remote mode)', () => {
  beforeEach(() => {
    mockRemoteAdminApi.overview.mockResolvedValue({
      adminEmail: 'admin@harbor.test',
      domain: 'harbor.test',
      userCount: 1,
      adminCount: 1,
      audit: [
        { id: 'a1', time: '2026-09-18T00:00:00Z', actor: 'admin', action: 'admin.user.created', detail: 'alpha' },
      ],
    })
    mockRemoteAdminApi.users.mockResolvedValue([
      {
        id: 'u1',
        email: 'alex@harbor.test',
        displayName: 'Alex',
        status: 'active',
        role: 'admin',
        storageUsedGB: 0.4,
        quotaGB: 10,
      },
    ])
    mockRemoteAdminApi.aliases.mockResolvedValue([
      { id: 'al1', address: 'support@harbor.test', forwardTo: 'alex@harbor.test' },
    ])
    mockRemoteAdminApi.audit.mockResolvedValue([
      { id: 'a1', time: '2026-09-18T00:00:00Z', actor: 'admin', action: 'admin.user.created', detail: 'alpha' },
    ])
    mockRemoteAdminApi.blockedSenders.mockResolvedValue(['spam@harbor.test'])
  })

  it('loads the live overview and shows the real domain', async () => {
    mount()
    expect(await screen.findByText('harbor.test')).toBeInTheDocument()
    expect(mockRemoteAdminApi.overview).toHaveBeenCalled()
  })

  it('lists mailboxes from the live users endpoint', async () => {
    mount()
    await clickTab(/mailboxes/i)
    expect(await screen.findByText('alex@harbor.test')).toBeInTheDocument()
    expect(mockRemoteAdminApi.users).toHaveBeenCalled()
  })

  it('lists aliases from the live aliases endpoint', async () => {
    mount()
    await clickTab(/aliases/i)
    expect(await screen.findByText('support@harbor.test')).toBeInTheDocument()
  })

  it('shows live blocked senders on the security tab', async () => {
    mount()
    await clickTab(/security/i)
    expect(await screen.findByText('spam@harbor.test')).toBeInTheDocument()
  })

  it('shows the audit log from the live audit endpoint', async () => {
    mount()
    await clickTab(/audit log/i)
    expect(await screen.findByText('admin.user.created')).toBeInTheDocument()
  })

  it('marks forwarders as Stalwart-managed in remote mode', async () => {
    mount()
    await clickTab(/forwarders/i)
    expect(await screen.findByText(/External forwarding rules are configured/)).toBeInTheDocument()
  })
})