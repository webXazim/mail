import { lazy, Suspense, type ReactNode } from 'react'
import { Navigate, Route, Routes, useLocation } from 'react-router-dom'

const LoginPage = lazy(() => import('./pages/LoginPage').then(module => ({ default: module.LoginPage })))
const MailLayout = lazy(() => import('./components/layout/MailLayout').then(module => ({ default: module.MailLayout })))
const MailListPage = lazy(() => import('./pages/MailListPage').then(module => ({ default: module.MailListPage })))
const ThreadPage = lazy(() => import('./pages/ThreadPage').then(module => ({ default: module.ThreadPage })))
const CalendarPage = lazy(() => import('./pages/CalendarPage').then(module => ({ default: module.CalendarPage })))

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