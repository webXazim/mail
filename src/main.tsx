import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { BrowserRouter } from 'react-router-dom'
import App from './App'
import { ErrorBoundary } from './components/ErrorBoundary'
import { MailProvider } from './state/mail/MailContext'
import { applyDensity, applyTheme, settingsApi } from './services/settings'
import './styles/tokens.css'
import './styles.css'

applyTheme(settingsApi.load().theme)
applyDensity(settingsApi.load().density)

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
