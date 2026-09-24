import { ApiError, apiFetch, tokenStore } from '../lib/api'

/**
 * Demo (fully client-side) mode is a build-time option only: set
 * VITE_DEMO_MODE=true in a local, uncommitted .env for offline development.
 * Production builds must leave it unset/false so the app can never fall back
 * to a fabricated session.
 */
export const isDemoAllowed = () => import.meta.env.VITE_DEMO_MODE === 'true'

const DEMO_KEY = 'cs-mail:demo'
const PROFILE_KEY = 'cs-mail:profile'

const clearProfileCache = () => {
  localStorage.removeItem(PROFILE_KEY)
  localStorage.removeItem('cs-mail:primary-identity')
  window.dispatchEvent(new Event('cs-mail-profile-cleared'))
}

if (!isDemoAllowed()) localStorage.removeItem(DEMO_KEY)

export type AuthUser = {
  id: string
  email: string
  display_name: string
  role: string
  platform_role?: 'user' | 'platform_support' | 'platform_admin'
  email_verified?: boolean
}

export type AuthResponse = {
  access: string
  /** Only present in demo mode; the live server sends refresh over HttpOnly cookie. */
  refresh?: string
  user: AuthUser
  recovery_code_used?: boolean
}

export type TwoFactorChallenge = {
  two_factor_required: true
  challenge_token: string
  expires_in: number
}

function demoAuth(email: string, name: string): AuthResponse {
  if (!isDemoAllowed()) throw new Error('Demo mode is disabled in this build')
  localStorage.removeItem('cs-mail:demo-removed')
  return {
    access: `demo.${email}`,
    refresh: `demo.${email}`,
    user: {
      id: 'demo-user',
      email,
      display_name: name || 'Demo User',
      role: 'admin',
    },
  }
}

export const authApi = {
  async login(email: string, password: string): Promise<AuthResponse | TwoFactorChallenge> {
    if (!email.includes('@') || password.length < 12) throw new Error('Invalid email or password')
    try {
      const result = await apiFetch<AuthResponse | TwoFactorChallenge>(
        '/api/auth/login',
        { method: 'POST', body: JSON.stringify({ email, password }) },
        { retry: false },
      )
      if ('two_factor_required' in result) {
        authApi.setDemo(false)
        return result
      }
      clearProfileCache()
      tokenStore.set(result.access)
      authApi.setDemo(false)
      return result
    } catch (cause) {
      if (!isDemoAllowed() || cause instanceof ApiError) throw cause
      const result = demoAuth(email, 'Demo User')
      tokenStore.set(result.access)
      authApi.setDemo(true)
      return result
    }
  },

  async verifyTwoFactor(challengeToken: string, code: string): Promise<AuthResponse> {
    const result = await apiFetch<AuthResponse>(
      '/api/auth/2fa/verify',
      {
        method: 'POST',
        body: JSON.stringify({ challenge_token: challengeToken, code }),
      },
      { retry: false },
    )
    clearProfileCache()
    tokenStore.set(result.access)
    authApi.setDemo(false)
    return result
  },

  async register(
    name: string,
    email: string,
    password: string,
  ): Promise<AuthResponse | { ok: boolean; message: string }> {
    if (!name.trim() || !email.includes('@') || password.length < 12)
      throw new Error('Please fill in all fields correctly')
    try {
      const result = await apiFetch<AuthResponse | { ok: boolean; message: string }>(
        '/api/auth/register',
        {
          method: 'POST',
          body: JSON.stringify({ name, email, password }),
        },
        { retry: false },
      )
      clearProfileCache()
      if ('access' in result && result.access) tokenStore.set(result.access)
      else tokenStore.clear()
      authApi.setDemo(false)
      return result
    } catch (cause) {
      if (!isDemoAllowed() || cause instanceof ApiError) throw cause
      const result = demoAuth(email, name)
      tokenStore.set(result.access)
      authApi.setDemo(true)
      return result
    }
  },

  async logout() {
    try {
      if (!authApi.isDemo()) {
        await apiFetch('/api/auth/logout', { method: 'POST' }, { retry: false })
      }
    } finally {
      tokenStore.clear()
      clearProfileCache()
      authApi.setDemo(false)
    }
  },

  async logoutAll() {
    try {
      if (!authApi.isDemo()) {
        await apiFetch('/api/auth/logout-all', { method: 'POST' }, { retry: false })
      }
    } finally {
      tokenStore.clear()
      clearProfileCache()
      authApi.setDemo(false)
    }
  },

  async me(): Promise<AuthUser> {
    return apiFetch<AuthUser>('/api/profile')
  },

  async requestPasswordReset(email: string) {
    return apiFetch<{ ok: boolean; message?: string }>(
      '/api/auth/forgot',
      { method: 'POST', body: JSON.stringify({ email }) },
      { retry: false },
    )
  },

  async resetPassword(token: string, password: string) {
    if (password.length < 12) throw new Error('Password must be at least 12 characters')
    return apiFetch<{ ok: boolean; message?: string }>(
      '/api/auth/reset',
      { method: 'POST', body: JSON.stringify({ token, password }) },
      { retry: false },
    )
  },

  async verifyEmail(token: string) {
    const result = await apiFetch<AuthResponse>(
      '/api/auth/verify',
      { method: 'POST', body: JSON.stringify({ token }) },
      { retry: false },
    )
    clearProfileCache()
    tokenStore.set(result.access)
    return { ok: true }
  },

  async resendVerification(email: string) {
    return apiFetch<{ ok: boolean; message?: string }>(
      '/api/auth/resend-verification',
      { method: 'POST', body: JSON.stringify({ email }) },
      { retry: false },
    )
  },

  isDemo: () => isDemoAllowed() && localStorage.getItem(DEMO_KEY) === 'true',
  setDemo: (value: boolean) => {
    if (value && isDemoAllowed()) localStorage.setItem(DEMO_KEY, 'true')
    else localStorage.removeItem(DEMO_KEY)
  },
}
