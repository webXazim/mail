import { beforeEach, describe, expect, it, vi } from 'vitest'
import { render, screen, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { MemoryRouter } from 'react-router-dom'
import { AdminPage } from './AdminPage'

const mockRemoteAdminApi = vi.hoisted(() => ({
  overview: vi.fn(),
  users: vi.fn(),
  usersPage: vi.fn(),
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
  forwarders: vi.fn(),
  createForwarder: vi.fn(),
  verifyForwarder: vi.fn(),
  setForwarderEnabled: vi.fn(),
  deleteForwarder: vi.fn(),
  domain: vi.fn(),
  updateDomain: vi.fn(),
  securityPolicy: vi.fn(),
  updateSecurityPolicy: vi.fn(),
  quarantine: vi.fn(),
  releaseQuarantine: vi.fn(),
  deleteQuarantine: vi.fn(),
  queue: vi.fn(),
  retryQueuedMessage: vi.fn(),
  cancelQueuedMessage: vi.fn(),
  diagnostics: vi.fn(),
  auditExport: vi.fn(),
  launchCertifications: vi.fn(),
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
      adminEmail: 'admin@cs-mail.test',
      domain: 'cs-mail.test',
      userCount: 1,
      adminCount: 1,
      auditEventCount: 1,
      providerHealthy: true,
    })
    mockRemoteAdminApi.users.mockResolvedValue([
      {
        id: 'u1',
        email: 'alex@cs-mail.test',
        displayName: 'Alex',
        status: 'active',
        role: 'admin',
        storageUsedGB: 0.4,
        quotaGB: 10,
      },
    ])
    mockRemoteAdminApi.usersPage.mockResolvedValue({
      users: [{
        id: 'u1',
        email: 'alex@cs-mail.test',
        displayName: 'Alex',
        status: 'active',
        role: 'admin',
        storageUsedGB: 0.4,
        quotaGB: 10,
      }],
      total: 1,
      limit: 100,
      offset: 0,
    })
    mockRemoteAdminApi.aliases.mockResolvedValue([
      { id: 'al1', address: 'support@cs-mail.test', forwardTo: 'alex@cs-mail.test' },
    ])
    mockRemoteAdminApi.audit.mockResolvedValue([
      { id: 'a1', time: '2026-09-18T00:00:00Z', actor: 'admin', action: 'admin.user.created', detail: 'alpha' },
    ])
    mockRemoteAdminApi.blockedSenders.mockResolvedValue(['spam@cs-mail.test'])
    mockRemoteAdminApi.forwarders.mockResolvedValue([
      { id: 'u1', from: 'alex@cs-mail.test', to: 'outside@example.net', enabled: true, verified: true, keepCopy: true },
    ])
    mockRemoteAdminApi.quarantine.mockResolvedValue([])
    mockRemoteAdminApi.domain.mockResolvedValue({
      domain: { domain: 'cs-mail.test', catchAllEnabled: false, catchAll: '', dnsZoneFile: 'cs-mail.test. IN MX 10 mail.cs-mail.test.' },
      dns: { mx: true, spf: true, dkim: true, dmarc: true },
    })
    mockRemoteAdminApi.securityPolicy.mockResolvedValue({
      requireTls: true, scanAttachments: true, spamThreshold: 5, retentionDays: 30, trashAutoPurge: true, dmarcPolicy: 'quarantine', blockedSenders: [],
      supported: { spamThreshold: true, retention: true, blockedSenders: true, requireTls: false, scanAttachments: false, dmarcPolicy: false, sso: false },
    })
    mockRemoteAdminApi.queue.mockResolvedValue({ messages: [], total: 0 })
    mockRemoteAdminApi.diagnostics.mockResolvedValue({
      database: true, mailProvider: true, queueTotal: 0, automationErrors: 0, addressSyncErrors: 0, provisioning: [],
    })
    mockRemoteAdminApi.launchCertifications.mockResolvedValue([])
  })

  it('loads the live overview and shows the real domain', async () => {
    mount()
    expect(await screen.findByText('cs-mail.test')).toBeInTheDocument()
    expect(mockRemoteAdminApi.overview).toHaveBeenCalled()
  })

  it('lists mailboxes from the live users endpoint', async () => {
    mount()
    await clickTab(/users/i)
    expect(await screen.findByText('alex@cs-mail.test')).toBeInTheDocument()
    expect(mockRemoteAdminApi.usersPage).toHaveBeenCalled()
  })

  it('lists aliases from the live aliases endpoint', async () => {
    mount()
    await clickTab(/aliases/i)
    expect(await screen.findByText('support@cs-mail.test')).toBeInTheDocument()
  })

  it('shows live blocked senders on the security tab', async () => {
    mount()
    await clickTab(/security/i)
    expect(await screen.findByText('spam@cs-mail.test')).toBeInTheDocument()
  })

  it('shows the audit log from the live audit endpoint', async () => {
    mount()
    await clickTab(/audit log/i)
    expect(await screen.findByText('admin.user.created')).toBeInTheDocument()
  })

  it('lists server-authoritative forwarders in remote mode', async () => {
    mount()
    await vi.waitFor(() => expect(mockRemoteAdminApi.forwarders).toHaveBeenCalled())
    await clickTab(/forwarders/i)
    expect(await screen.findByRole('heading', { name: 'Forwarders' })).toBeInTheDocument()
    expect(mockRemoteAdminApi.forwarders).toHaveBeenCalled()
  })
})
