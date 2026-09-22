import { Link } from 'react-router-dom'
import {
  Archive,
  ArrowRight,
  Bell,
  CalendarClock,
  FileText,
  Filter,
  Fingerprint,
  FolderTree,
  HardDriveDownload,
  Inbox,
  Keyboard,
  LayoutGrid,
  Search,
  ShieldCheck,
  Swords,
  Zap,
} from 'lucide-react'

const groups = [
  {
    heading: 'Triage & focus',
    icon: Zap,
    items: [
      {
        icon: Inbox,
        title: 'Conversation view',
        body: 'Threads stay together, so context survives the cc list. Split-pane reading on wide screens.',
      },
      {
        icon: Swords,
        title: 'Swipe to triage',
        body: 'Archive or trash with a thumb-stroke on mobile. Power tools that feel calm.',
      },
      {
        icon: Keyboard,
        title: 'Keyboard power',
        body: 'j/k, Enter, e, #, x, s, z — a Gmail-grade grammar for people who live in the inbox.',
      },
      {
        icon: Filter,
        title: 'Filters & routing',
        body: 'Inbox rules that sort, label, forward, and snooze on arrival. Zero-tolerance spam defense.',
      },
    ],
  },
  {
    heading: 'Find & manage',
    icon: Search,
    items: [
      {
        icon: Search,
        title: 'Operator search',
        body: 'from:, to:, subject:, in:trash, is:unread, has:attachment — precise queries, instant results.',
      },
      {
        icon: Archive,
        title: 'Archive, labels & folders',
        body: 'A flexible filing system — custom folders, cross-labels, and an Archive that is really an archive.',
      },
      {
        icon: CalendarClock,
        title: 'Snooze & schedule',
        body: 'Push messages to tomorrow, or send your reply on a schedule matching your timezone.',
      },
      {
        icon: FileText,
        title: 'Drafts that follow you',
        body: 'Autosaved drafts, signatures, and identities for every role you play.',
      },
    ],
  },
  {
    heading: 'Trust & control',
    icon: ShieldCheck,
    items: [
      {
        icon: Fingerprint,
        title: 'Two-factor by default',
        body: 'MFA, per-session controls, and sign-out-from-anywhere baked into the security center.',
      },
      {
        icon: HardDriveDownload,
        title: 'Your data, yours',
        body: 'Full export, 30-day retention after cancellation, and no advertising-grade tracking.',
      },
      {
        icon: LayoutGrid,
        title: 'Admin center',
        body: 'Mailboxes, aliases, forwarders, domain, and routing under one panel for teams.',
      },
      {
        icon: Bell,
        title: 'Notifications that respect you',
        body: 'Desktop alerts, digests on your schedule, and per-sender quiet hours.',
      },
    ],
  },
]

export function FeaturesPage() {
  return (
    <div className="site-page">
      <section className="site-page__head">
        <p className="eyebrow">Features</p>
        <h1>
          Everything a serious inbox
          <br />
          should already be.
        </h1>
        <p className="site-page__intro">
          CS Mail is built for people who treat email as work — not as a notification feed.
        </p>
      </section>

      {groups.map((group) => {
        const GroupIcon = group.icon
        return (
          <section className="feature-group" key={group.heading}>
            <header className="feature-group__head">
              <span className="feature-group__icon">
                <GroupIcon size={16} />
              </span>
              <h2>{group.heading}</h2>
            </header>
            <div className="feature-grid">
              {group.items.map((item) => {
                const Icon = item.icon
                return (
                  <article className="feature-card" key={item.title}>
                    <span className="feature-card__icon">
                      <Icon size={16} />
                    </span>
                    <h3>{item.title}</h3>
                    <p>{item.body}</p>
                  </article>
                )
              })}
            </div>
          </section>
        )
      })}

      <section className="home-band">
        <div>
          <p className="eyebrow">Folders are welcome too</p>
          <h2>
            Bring your{' '}
            <span>
              <FolderTree size={15} />
              existing workflow
            </span>{' '}
            as-is.
          </h2>
        </div>
        <Link to="/pricing" className="secondary-button">
          View pricing <ArrowRight size={14} />
        </Link>
      </section>
    </div>
  )
}
