import { apiBase } from './backend'
export const authApi = {
  async login(email: string, password: string) {
    if (apiBase) { const response = await fetch(`${apiBase}/auth/login`, { method: 'POST', credentials: 'include', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ email, password }) }); if (!response.ok) throw new Error('Invalid email or password'); return response.json() }
    if (!email.includes('@') || password.length < 6) throw new Error('Invalid email or password')
    return { user: { email }, demo: true }
  },
  async logout() { if (apiBase) await fetch(`${apiBase}/auth/logout`, { method: 'POST', credentials: 'include' }); localStorage.removeItem('harbor-mail:session') },
}
