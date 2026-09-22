import { Link } from 'react-router-dom'
import {
  ArrowRight,
  FileLock2,
  Fingerprint,
  LockKeyhole,
  Server,
  ShieldCheck,
  Swords,
} from 'lucide-react'

const pillars = [
  {
    icon: LockKeyhole,
    title: 'Encryption everywhere',
    body: 'TLS 1.2+ for everything in transit. AES-256 at rest with hardware-backed keys, rotated on schedule.',
  },
  {
    icon: Fingerprint,
    title: 'Least privilege access',
    body: 'No engineer holds standing access to production data. MFA, IP allowlisting, and a quarterly access review.',
  },
  {
    icon: Server,
    title: 'Hardened infrastructure',
    body: 'Isolated, immutable infrastructure across availability zones. Continuously scanned and patched in hours, not weeks.',
  },
  {
    icon: Swords,
    title: 'Testing that never stops',
    body: 'Annual third-party penetration tests and continuous automated security checks against the production posture.',
  },
]

const badges = [
  { label: 'Encrypted in transit', detail: 'TLS 1.2+' },
  { label: 'Encrypted at rest', detail: 'AES-256' },
  { label: 'Compliance', detail: 'SOC 2 Type II' },
  { label: 'Service health', detail: 'Live status' },
]

export function SecurityPage() {
  return (
    <div className="site-page">
      <section className="site-page__head">
        <p className="eyebrow">Security</p>
        <h1>
          Trust is a feature.
          <br />
          Security is the default.
        </h1>
        <p className="site-page__intro">
          Every message you send and receive is protected by the same controls we run our own
          operations under.
        </p>
      </section>

      <section className="security-badges" aria-label="Certifications">
        {badges.map((badge) => (
          <article className="security-badge" key={badge.label}>
            <span className="security-badge__icon">
              <ShieldCheck size={16} />
            </span>
            <div>
              <strong>{badge.detail}</strong>
              <small>{badge.label}</small>
            </div>
          </article>
        ))}
      </section>

      <section className="feature-grid security-pillars" aria-label="Security practices">
        {pillars.map((pillar) => {
          const Icon = pillar.icon
          return (
            <article className="feature-card" key={pillar.title}>
              <span className="feature-card__icon">
                <Icon size={16} />
              </span>
              <h2>{pillar.title}</h2>
              <p>{pillar.body}</p>
            </article>
          )
        })}
      </section>

      <section className="home-band">
        <div>
          <p className="eyebrow">The fine print, in the open</p>
          <h2>Read our security posture and compliance docs.</h2>
        </div>
        <Link to="/legal/security" className="secondary-button">
          <FileLock2 size={14} />
          Security &amp; Compliance <ArrowRight size={14} />
        </Link>
      </section>
    </div>
  )
}
