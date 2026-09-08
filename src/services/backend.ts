export type BackendKind = 'local' | 'remote'

const configured = import.meta.env.VITE_MAIL_BACKEND as string | undefined

export const backendKind: BackendKind = configured === 'remote' ? 'remote' : 'local'
export const apiBase = import.meta.env.VITE_API_URL as string | undefined