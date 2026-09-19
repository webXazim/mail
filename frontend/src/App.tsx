import { lazy, Suspense, useEffect, useState, type ReactNode } from 'react'
import { Navigate, Route, Routes, useLocation } from 'react-router-dom'
import { bootstrapSession } from './lib/api'
import { isDemoAllowed } from './services/auth'
import { profileApi, useRole } from './services/profile'

const LoginPage = lazy(() =>
  import('./pages/LoginPage').then((module) => ({ default: module.LoginPage })),
)
const CreateAccountPage = lazy(() =>
  import('./pages/CreateAccountPage').then((module) => ({ default: module.CreateAccountPage })),
)
const ForgotPasswordPage = lazy(() =>
  import('./pages/ForgotPasswordPage').then((module) => ({ default: module.ForgotPasswordPage })),
)
const ResetPasswordPage = lazy(() =>
  import('./pages/ResetPasswordPage').then((module) => ({ default: module.ResetPasswordPage })),
)
const VerifyEmailPage = lazy(() =>
  import('./pages/VerifyEmailPage').then((module) => ({ default: module.VerifyEmailPage })),
)
const HomePage = lazy(() =>
  import('./pages/HomePage').then((module) => ({ default: module.HomePage })),
)
const FeaturesPage = lazy(() =>
  import('./pages/FeaturesPage').then((module) => ({ default: module.FeaturesPage })),
)
const SecurityPage = lazy(() =>
  import('./pages/SecurityPage').then((module) => ({ default: module.SecurityPage })),
)
const PublicPricingPage = lazy(() =>
  import('./pages/PublicPricingPage').then((module) => ({ default: module.PublicPricingPage })),
)
const HelpPage = lazy(() =>
  import('./pages/HelpPage').then((module) => ({ default: module.HelpPage })),
)
const ContactPage = lazy(() =>
  import('./pages/ContactPage').then((module) => ({ default: module.ContactPage })),
)
const StatusPage = lazy(() =>
  import('./pages/StatusPage').then((module) => ({ default: module.StatusPage })),
)
const PublicLayout = lazy(() =>
  import('./components/layout/PublicLayout').then((module) => ({ default: module.PublicLayout })),
)
const MailLayout = lazy(() =>
  import('./components/layout/MailLayout').then((module) => ({ default: module.MailLayout })),
)
const MailListPage = lazy(() =>
  import('./pages/MailListPage').then((module) => ({ default: module.MailListPage })),
)
const SearchPage = lazy(() =>
  import('./pages/SearchPage').then((module) => ({ default: module.SearchPage })),
)
const ThreadPage = lazy(() =>
  import('./pages/ThreadPage').then((module) => ({ default: module.ThreadPage })),
)
const CalendarPage = lazy(() =>
  import('./pages/CalendarPage').then((module) => ({ default: module.CalendarPage })),
)
const ContactsPage = lazy(() =>
  import('./pages/ContactsPage').then((module) => ({ default: module.ContactsPage })),
)
const AdminPage = lazy(() =>
  import('./pages/AdminPage').then((module) => ({ default: module.AdminPage })),
)
const AdminBillingPage = lazy(() =>
  import('./pages/AdminBillingPage').then((module) => ({ default: module.AdminBillingPage })),
)
const SettingsPage = lazy(() =>
  import('./pages/SettingsPage').then((module) => ({ default: module.SettingsPage })),
)
const NotificationsPage = lazy(() =>
  import('./pages/NotificationsPage').then((module) => ({ default: module.NotificationsPage })),
)
const LabelsPage = lazy(() =>
  import('./pages/LabelsPage').then((module) => ({ default: module.LabelsPage })),
)
const TemplatesPage = lazy(() =>
  import('./pages/TemplatesPage').then((module) => ({ default: module.TemplatesPage })),
)
const BillingPage = lazy(() =>
  import('./pages/BillingPage').then((module) => ({ default: module.BillingPage })),
)
const InvoicePage = lazy(() =>
  import('./pages/InvoicePage').then((module) => ({ default: module.InvoicePage })),
)
const AuditLogPage = lazy(() =>
  import('./pages/AuditLogPage').then((module) => ({ default: module.AuditLogPage })),
)
const PricingPage = lazy(() =>
  import('./pages/PricingPage').then((module) => ({ default: module.PricingPage })),
)
const LegalPage = lazy(() =>
  import('./pages/LegalPage').then((module) => ({ default: module.LegalPage })),
)
const NotFoundPage = lazy(() =>
  import('./pages/NotFoundPage').then((module) => ({ default: module.NotFoundPage })),
)

const sessionKey = 'harbor-mail:demo'

function RequireAuth({ children }: { children: ReactNode }) {
  const location = useLocation()
  const [ready, setReady] = useState(false)
  const [ok, setOk] = useState(false)
  const demo = isDemoAllowed() && localStorage.getItem(sessionKey) === 'true'
  useEffect(() => {
    let alive = true
    bootstrapSession().then((v) => {
      if (alive) {
        setOk(v)
        setReady(true)
      }
    })
    return () => {
      alive = false
    }
  }, [])
  if (!ready)
    return (
      <div className="route-loader">
        <div className="loading-spinner" />
      </div>
    )
  if (!ok && !demo) return <Navigate to="/login" replace state={{ from: location.pathname }} />
  return children
}

function RequireRole({ roles, children }: { roles: string[]; children: ReactNode }) {
  const location = useLocation()
  const role = useRole()
  const [checked, setChecked] = useState(false)
  useEffect(() => {
    let alive = true
    const finish = () => {
      if (alive) setChecked(true)
    }
    profileApi.refresh().then(finish).catch(finish)
    return () => {
      alive = false
    }
  }, [])
  if (!checked)
    return (
      <div className="route-loader">
        <div className="loading-spinner" />
      </div>
    )
  if (!role) return <Navigate to="/login" replace state={{ from: location.pathname }} />
  if (!roles.includes(role)) return <Navigate to="/mail/inbox" replace />
  return children
}

export default function App() {
  return (
    <Suspense
      fallback={
        <div className="route-loader">
          <div className="loading-spinner" />
        </div>
      }
    >
      <Routes>
        <Route path="/login" element={<LoginPage />} />
        <Route path="/create-account" element={<CreateAccountPage />} />
        <Route path="/forgot-password" element={<ForgotPasswordPage />} />
        <Route path="/reset-password" element={<ResetPasswordPage />} />
        <Route path="/verify-email" element={<VerifyEmailPage />} />
        <Route element={<PublicLayout />}>
          <Route index element={<HomePage />} />
          <Route path="features" element={<FeaturesPage />} />
          <Route path="security" element={<SecurityPage />} />
          <Route path="pricing" element={<PublicPricingPage />} />
          <Route path="help" element={<HelpPage />} />
          <Route path="contact" element={<ContactPage />} />
          <Route path="status" element={<StatusPage />} />
        </Route>
        <Route path="/legal/terms" element={<LegalPage docId="terms" />} />
        <Route path="/legal/privacy" element={<LegalPage docId="privacy" />} />
        <Route path="/legal/aup" element={<LegalPage docId="aup" />} />
        <Route path="/legal/security" element={<LegalPage docId="security" />} />
        <Route path="/legal/abuse" element={<LegalPage docId="abuse" />} />
        <Route path="/legal" element={<Navigate to="/legal/terms" replace />} />
        <Route
          path="/mail"
          element={
            <RequireAuth>
              <MailLayout />
            </RequireAuth>
          }
        >
          <Route path="calendar" element={<CalendarPage />} />
          <Route path="contacts" element={<ContactsPage />} />
          <Route
            path="admin"
            element={
              <RequireRole roles={['admin']}>
                <AdminPage />
              </RequireRole>
            }
          />
          <Route
            path="admin/billing"
            element={
              <RequireRole roles={['admin']}>
                <AdminBillingPage />
              </RequireRole>
            }
          />
          <Route path="settings" element={<SettingsPage />} />
          <Route path="notifications" element={<NotificationsPage />} />
          <Route path="labels" element={<LabelsPage />} />
          <Route path="templates" element={<TemplatesPage />} />
          <Route path="billing" element={<BillingPage />} />
          <Route path="billing/invoices/:invoiceId" element={<InvoicePage />} />
          <Route path="audit-log" element={<AuditLogPage />} />
          <Route path="pricing" element={<PricingPage />} />
          <Route path="search" element={<SearchPage />} />
          <Route path=":folder" element={<MailListPage />} />
          <Route path=":folder/thread/:mailId" element={<ThreadPage />} />
          <Route path="folders/:folderId" element={<MailListPage />} />
          <Route path="folders/:folderId/thread/:mailId" element={<ThreadPage />} />
          <Route path="*" element={<NotFoundPage />} />
        </Route>
        <Route path="*" element={<NotFoundPage full />} />
      </Routes>
    </Suspense>
  )
}
