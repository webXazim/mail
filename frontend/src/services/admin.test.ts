import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { remoteAdminApi } from './admin'

const jsonResponse = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } })

describe('remoteAdminApi (live mode)', () => {
  beforeEach(() => {
    vi.stubGlobal('fetch', vi.fn())
  })

  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('maps backend users into mailbox accounts measured in GB', async () => {
    vi.mocked(fetch).mockResolvedValue(
      jsonResponse({
        users: [
          {
            id: 'u1',
            email: 'alex@cs-mail.test',
            display_name: 'Alex',
            role: 'admin',
            plan: 'pro',
            quota_bytes: 5 * 1024 * 1024 * 1024,
            mail_account_id: 'm',
            onboarded: true,
            created_at: '2026-09-01T00:00:00Z',
            storage_used_bytes: 1024 * 1024 * 1024,
            storage_pct: 20,
          },
        ],
      }),
    )

    const [row] = await remoteAdminApi.users()
    expect(row.email).toBe('alex@cs-mail.test')
    expect(row.role).toBe('admin')
    expect(row.status).toBe('active')
    expect(row.quotaGB).toBe(5)
    expect(row.storageUsedGB).toBe(1)
  })

  it('maps the backend billing role onto the mailbox role model', async () => {
    vi.mocked(fetch).mockResolvedValue(
      jsonResponse({
        users: [
          { id: 'u1', email: 'bill@cs-mail.test', display_name: 'B', role: 'billing', plan: 'solo', quota_bytes: 1073741824, mail_account_id: null, onboarded: false, created_at: '2026-09-01T00:00:00Z', storage_used_bytes: 0, storage_pct: 0 },
        ],
      }),
    )

    const [row] = await remoteAdminApi.users()
    expect(row.role).toBe('billing')
    expect(row.quotaGB).toBe(1)
  })

  it('posts the create-user body to /api/admin/users', async () => {
    const fetchMock = vi.mocked(fetch)
    fetchMock.mockResolvedValue(jsonResponse({ ok: true }))

    await remoteAdminApi.createUser({
      email: 'nora@cs-mail.test',
      displayName: 'Nora',
      password: 'Strong-Pass!1',
      quotaGB: 3,
    })

    const [url, init] = fetchMock.mock.calls[0]
    expect(url).toBe('/api/admin/users')
    expect(init?.method).toBe('POST')
    expect(JSON.parse(String(init?.body))).toMatchObject({
      email: 'nora@cs-mail.test',
      display_name: 'Nora',
      role: 'member',
      quota_bytes: 3 * 1024 * 1024 * 1024,
    })
  })

  it('maps raw audit detail objects into readable strings', async () => {
    vi.mocked(fetch).mockResolvedValue(
      jsonResponse({
        entries: [
          {
            id: 'e1',
            time: '2026-09-18T00:00:00Z',
            actor: 'admin',
            action: 'admin.user.created',
            detail: { user_id: 'u1', email: 'nora@cs-mail.test' },
          },
        ],
      }),
    )

    const [entry] = await remoteAdminApi.audit()
    expect(entry.action).toBe('admin.user.created')
    expect(entry.detail).toContain('nora@cs-mail.test')
  })

  it('flattens alias rows for the admin table', async () => {
    vi.mocked(fetch).mockResolvedValue(
      jsonResponse({
        aliases: [{ id: 'al1', address: 'support@cs-mail.test', domain: 'cs-mail.test', source: 'support', forwardTo: 'alex@cs-mail.test' }],
      }),
    )

    const [alias] = await remoteAdminApi.aliases()
    expect(alias).toEqual({ id: 'al1', address: 'support@cs-mail.test', forwardTo: 'alex@cs-mail.test' })
  })

  it('maps the server-authoritative overview counters and provider health', async () => {
    vi.mocked(fetch).mockResolvedValue(
      jsonResponse({
        admin: { id: 'a1', email: 'admin@cs-mail.test' },
        domain: 'cs-mail.test',
        user_count: 3,
        admin_count: 1,
        audit_events: 42,
        provider_healthy: true,
      }),
    )

    const result = await remoteAdminApi.overview()
    expect(result.domain).toBe('cs-mail.test')
    expect(result.userCount).toBe(3)
    expect(result.adminCount).toBe(1)
    expect(result.auditEventCount).toBe(42)
    expect(result.providerHealthy).toBe(true)
  })
})