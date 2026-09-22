import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { BrowserRouter } from 'react-router-dom'
import App from './App'
import { ErrorBoundary } from './components/ErrorBoundary'
import { MailProvider } from './state/mail/MailContext'
import {
  applyAccent,
  applyDensity,
  applyRadius,
  applyTheme,
  settingsApi,
} from './services/settings'
import { initTooltips } from './lib/tooltips'
import './services/ws'
import './styles/tokens.css'
import './styles.css'

applyTheme(settingsApi.load().theme)
applyDensity(settingsApi.load().density)
applyRadius(settingsApi.load().radius)
applyAccent(settingsApi.load().accent)

// API-first hydration: replace cached settings with the server copy when signed in.
void settingsApi.refresh()

window.addEventListener('cs-mail-resource-changed', (incoming) => {
  const detail = (incoming as CustomEvent<{ payload?: { resource?: string } }>).detail
  if (detail?.payload?.resource === 'settings') void settingsApi.refresh()
})

if (import.meta.env.PROD && 'serviceWorker' in navigator) {
  window.addEventListener('load', () => {
    void navigator.serviceWorker.register('/sw.js').catch(() => {})
  })
}

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <ErrorBoundary>
      <BrowserRouter>
        <MailProvider>
          <App />
        </MailProvider>
      </BrowserRouter>
    </ErrorBoundary>
  </StrictMode>,
)

initTooltips()
