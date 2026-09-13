import { useSyncExternalStore } from 'react'

const MOBILE_QUERY = '(max-width: 700px)'

function canMatch() {
  return typeof window !== 'undefined' && typeof window.matchMedia === 'function'
}

const subscribe = (callback: () => void) => {
  if (!canMatch()) return () => {}
  const mql = window.matchMedia(MOBILE_QUERY)
  mql.addEventListener('change', callback)
  return () => mql.removeEventListener('change', callback)
}

const getSnapshot = () => (canMatch() ? window.matchMedia(MOBILE_QUERY).matches : false)

const getServerSnapshot = () => false

export function useIsMobile() {
  return useSyncExternalStore(subscribe, getSnapshot, getServerSnapshot)
}
