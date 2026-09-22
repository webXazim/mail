import { apiFetch } from '../lib/api'

export type AccountSession = {
  id: string
  device: string
  user_agent: string
  ip: string
  created_at: string
  last_used_at: string
  expires_at: string
  current: boolean
}

export type TwoFactorStatus = {
  enabled: boolean
  enabled_at: string | null
  recovery_codes_remaining: number
  setup_pending_until: string | null
}

export type TwoFactorSetup = {
  secret: string
  otpauth_uri: string
  qr_svg: string
  expires_at: string
}

export const securityApi = {
  async changePassword(currentPassword: string, newPassword: string) {
    return apiFetch<{ ok: boolean; message: string }>(
      '/api/account/change-password',
      {
        method: 'POST',
        body: JSON.stringify({
          current_password: currentPassword,
          new_password: newPassword,
        }),
      },
      { retry: false },
    )
  },

  async sessions(): Promise<AccountSession[]> {
    const result = await apiFetch<{ sessions: AccountSession[] }>('/api/account/sessions')
    return result.sessions
  },

  async revokeSession(id: string) {
    return apiFetch<{ ok: boolean }>(`/api/account/sessions/${encodeURIComponent(id)}`, {
      method: 'DELETE',
    })
  },

  async revokeOtherSessions() {
    return apiFetch<{ ok: boolean; revoked: number }>('/api/account/sessions/revoke-others', {
      method: 'POST',
    })
  },

  async twoFactorStatus(): Promise<TwoFactorStatus> {
    return apiFetch<TwoFactorStatus>('/api/account/2fa/status')
  },

  async startTwoFactorSetup(currentPassword: string): Promise<TwoFactorSetup> {
    return apiFetch<TwoFactorSetup>(
      '/api/account/2fa/setup',
      {
        method: 'POST',
        body: JSON.stringify({ current_password: currentPassword }),
      },
      { retry: false },
    )
  },

  async confirmTwoFactor(code: string) {
    return apiFetch<{ ok: boolean; recovery_codes: string[]; message: string }>(
      '/api/account/2fa/confirm',
      { method: 'POST', body: JSON.stringify({ code }) },
      { retry: false },
    )
  },

  async disableTwoFactor(currentPassword: string, code: string) {
    return apiFetch<{ ok: boolean; message: string }>(
      '/api/account/2fa/disable',
      {
        method: 'POST',
        body: JSON.stringify({ current_password: currentPassword, code }),
      },
      { retry: false },
    )
  },

  async regenerateRecoveryCodes(currentPassword: string, code: string) {
    return apiFetch<{ ok: boolean; recovery_codes: string[] }>(
      '/api/account/2fa/recovery-codes/regenerate',
      {
        method: 'POST',
        body: JSON.stringify({ current_password: currentPassword, code }),
      },
      { retry: false },
    )
  },
}
