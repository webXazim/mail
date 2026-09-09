import { lazy, Suspense, type ReactNode } from 'react'
import { Navigate, Route, Routes, useLocation } from 'react-router-dom'

const LoginPage = lazy(() => import('./pages/LoginPage').then(module => ({ default: module.LoginPage })))
const MailLayout = lazy(() => import('./components/layout/MailLayout').then(module => ({ default: module.MailLayout })))
const MailListPage = lazy(() => import('./pages/MailListPage').then(module => ({ default: module.MailListPage })))
const ThreadPage = lazy(() => import('./pages/ThreadPage').then(module => ({ default: module.ThreadPage })))
const CalendarPage = lazy(() => import('./pages/CalendarPage').then(module => ({ default: module.CalendarPage })))
const ContactsPage = lazy(() => import('./pages/ContactsPage').then(module => ({ default: module.ContactsPage })))
const AdminPage = lazy(() => import('./pages/AdminPage').then(module => ({ default: module.AdminPage })))
const SettingsPage = lazy(() => import('./pages/SettingsPage').then(module => ({ default: module.SettingsPage })))
const BillingPage = lazy(() => import('./pages/BillingPage').then(module => ({ default: module.BillingPage })))
const PricingPage = lazy(() => import('./pages/PricingPage').then(module => ({ default: module.PricingPage })))

const sessionKey = 'harbor-mail:session'

function RequireAuth({ children }: { children: ReactNode }) {
  const location = useLocation()
  if (!localStorage.getItem(sessionKey)) return <Navigate to="/login" replace state={{ from: location.pathname }} />
  return children
}

export default function App() {
  return (
    <Suspense fallback={<div className="route-loader"><div className="loading-spinner" /></div>}>
      <Routes>
        <Route path="/login" element={<LoginPage />} />
        <Route
          path="/mail"
          element={(
            <RequireAuth>
              <MailLayout />
            </RequireAuth>
          )}
        >
          <Route path="calendar" element={<CalendarPage />} />
          <Route path="contacts" element={<ContactsPage />} />
          <Route path="admin" element={<AdminPage />} />
          <Route path="settings" element={<SettingsPage />} />
          <Route path="billing" element={<BillingPage />} />
          <Route path="pricing" element={<PricingPage />} />
          <Route path=":folder" element={<MailListPage />} />
          <Route path=":folder/thread/:mailId" element={<ThreadPage />} />
          <Route path="folders/:folderId" element={<MailListPage />} />
          <Route path="folders/:folderId/thread/:mailId" element={<ThreadPage />} />
        </Route>
        <Route path="*" element={<Navigate to="/mail/inbox" replace />} />
      </Routes>
    </Suspense>
  )
}