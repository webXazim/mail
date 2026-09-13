export const authApi = {
  async login(email: string, password: string) {
    if (!email.includes('@') || password.length < 6) throw new Error('Invalid email or password')
    return { user: { email }, demo: true }
  },
  async logout() {
    localStorage.removeItem('harbor-mail:session')
  },
  async requestPasswordReset(_email: string) {
    return { ok: true, demo: true }
  },
  async resetPassword(_token: string, password: string) {
    if (password.length < 6) throw new Error('Password must be at least 6 characters')
    return { ok: true }
  },
  async verifyEmail(_token: string) {
    return { ok: true }
  },
  async resendVerification(_email: string) {
    return { ok: true, demo: true }
  },
}