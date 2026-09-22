export type LegalSection = { id: string; heading: string; body: string[] }

export type LegalDoc = {
  id: string
  title: string
  updated: string
  intro: string
  sections: LegalSection[]
}

const updated = '21 September 2026'
const legalEmail = 'legal@crescentsphere.com'
const privacyEmail = 'privacy@crescentsphere.com'
const abuseEmail = 'abuse@crescentsphere.com'
const securityEmail = 'security@crescentsphere.com'

const terms: LegalDoc = {
  id: 'terms',
  title: 'Terms of Service',
  updated,
  intro:
    'These Terms of Service govern access to and use of CS Mail and its related services. By creating an account or using the service, you agree to these terms and to the policies referenced below.',
  sections: [
    {
      id: 'eligibility',
      heading: '1. Eligibility and accounts',
      body: [
        'You must be legally capable of entering into a binding agreement in your jurisdiction. Information supplied during registration must be accurate and kept current.',
        'You are responsible for protecting your credentials and for activity performed through your account. Contact us promptly if you believe an account has been compromised.',
      ],
    },
    {
      id: 'acceptable-use',
      heading: '2. Acceptable use',
      body: [
        'You may not use CS Mail for spam, phishing, malware distribution, harassment, unlawful activity, infringement, credential theft, or attempts to disrupt or gain unauthorized access to systems or accounts.',
        'The Acceptable Use Policy is part of these Terms. We may restrict or suspend activity when necessary to protect users, mail reputation, service availability, or comply with applicable law.',
      ],
    },
    {
      id: 'content',
      heading: '3. Your content',
      body: [
        'You retain ownership of messages, attachments, contacts and other content you submit. You authorize us to process that content only as needed to provide, secure, maintain and support the service.',
        'You are responsible for ensuring that content you send or store is lawful and that you have the rights required to use it.',
      ],
    },
    {
      id: 'availability',
      heading: '4. Service availability',
      body: [
        'Service health and incident information is published on the CS Mail Status page when available. Maintenance, upstream failures, abuse mitigation or security work may temporarily affect access.',
        'Any specific service-level commitment applies only when it is expressly included in a separate agreement for your account.',
      ],
    },
    {
      id: 'billing',
      heading: '5. Plans and billing',
      body: [
        'Paid plan prices, included limits and the active billing method are shown in the product before an order is submitted. Charges, renewals, refunds and cancellation rights are governed by the terms shown for the applicable order and by mandatory law.',
        'We may change future pricing or plan limits after reasonable notice. Changes do not retroactively alter already completed billing periods unless required by law.',
      ],
    },
    {
      id: 'termination',
      heading: '6. Suspension, cancellation and deletion',
      body: [
        'You may cancel or request deletion through the controls made available in CS Mail. We may suspend or terminate access for serious abuse, security risk, non-payment, legal requirements or material breach of these Terms.',
        'Deletion is subject to operational backup rotation and any retention that is legally required. The product should not be used as the sole copy of information you are required to preserve.',
      ],
    },
    {
      id: 'disclaimer',
      heading: '7. Disclaimer and liability',
      body: [
        'To the extent permitted by applicable law, CS Mail is provided on an as-available basis without warranties that cannot be disclaimed by contract.',
        'Nothing in these Terms excludes liability that cannot legally be excluded or limited. Any contractual limitation that applies to a paid account will be interpreted subject to applicable consumer and commercial law.',
      ],
    },
    {
      id: 'changes',
      heading: '8. Changes',
      body: [
        'We may update these Terms to reflect product, security, legal or operational changes. Material changes will be communicated through an appropriate service channel before they take effect where required.',
      ],
    },
    {
      id: 'contact',
      heading: '9. Contact',
      body: [`Questions about these Terms can be sent to ${legalEmail}.`],
    },
  ],
}

const privacy: LegalDoc = {
  id: 'privacy',
  title: 'Privacy Policy',
  updated,
  intro:
    'This Privacy Policy explains the categories of information CS Mail processes to operate the service and the controls available to users. It does not claim certifications, hosting regions or subprocessors that are not explicitly published by the service operator.',
  sections: [
    {
      id: 'information',
      heading: '1. Information we process',
      body: [
        'Account data may include your name, email address, authentication records, plan, quota and support history.',
        'Mail data includes message content, recipients, attachments, mailbox metadata and delivery information needed to store, search, send and receive email.',
        'Operational data may include IP address, user agent, timestamps, security events, audit records and diagnostic information used to protect and maintain the service.',
      ],
    },
    {
      id: 'purpose',
      heading: '2. Why we process information',
      body: [
        'We process information to provide mailbox functionality, authenticate users, prevent abuse, enforce plan limits, deliver support, maintain reliability and meet applicable legal obligations.',
        'CS Mail does not require selling personal information to advertisers in order to provide the service.',
      ],
    },
    {
      id: 'sharing',
      heading: '3. Service providers and disclosure',
      body: [
        'Information may be processed by infrastructure, payment, security or support providers when those services are used to operate CS Mail. Access should be limited to what is necessary for the relevant function.',
        'Information may also be disclosed when required by valid legal process or when reasonably necessary to investigate abuse, fraud, security incidents or threats to users and systems.',
      ],
    },
    {
      id: 'security',
      heading: '4. Security',
      body: [
        'CS Mail uses authenticated sessions, password hashing, role-based authorization, audit logging, rate limits and transport security controls in its production architecture. No internet service can guarantee absolute security.',
        'Security-sensitive credentials and deployment secrets should be stored outside the source repository and restricted to the services that need them.',
      ],
    },
    {
      id: 'retention',
      heading: '5. Retention and deletion',
      body: [
        'Operational records are retained only for periods needed for service delivery, security, dispute handling, backup rotation and legal obligations. Different data categories may therefore have different retention periods.',
        'Account deletion removes active account data through the product workflow. Backup copies expire through the normal backup-retention process rather than being edited in place.',
      ],
    },
    {
      id: 'rights',
      heading: '6. Your choices and rights',
      body: [
        'Depending on your jurisdiction, you may have rights to access, correct, export, object to processing of, or delete personal information. Product controls should be used where available.',
        `Privacy requests that cannot be completed in-product can be sent to ${privacyEmail}.`,
      ],
    },
    {
      id: 'cookies',
      heading: '7. Cookies and local storage',
      body: [
        'CS Mail uses a secure session cookie for authentication. Browser storage may also be used for presentation preferences, cached interface state and an explicitly enabled development/demo mode; authenticated server data remains authoritative.',
      ],
    },
    {
      id: 'contact',
      heading: '8. Contact',
      body: [`Privacy questions can be sent to ${privacyEmail}.`],
    },
  ],
}

const aup: LegalDoc = {
  id: 'aup',
  title: 'Acceptable Use Policy',
  updated,
  intro:
    'This Acceptable Use Policy protects CS Mail users, recipients, infrastructure and mail reputation. It supplements the Terms of Service.',
  sections: [
    {
      id: 'prohibited',
      heading: '1. Prohibited activity',
      body: [
        'Do not use the service for phishing, malware, credential harvesting, unlawful threats, harassment, impersonation, infringement, unauthorized access, denial-of-service activity or attempts to bypass service security controls.',
      ],
    },
    {
      id: 'spam',
      heading: '2. Spam and bulk mail',
      body: [
        'Do not send unsolicited bulk or deceptive email, use purchased or harvested recipient lists, falsify sender identity, or continue mailing recipients who have validly opted out.',
        'Senders are responsible for complying with the anti-spam and electronic-communications laws that apply to their recipients and use case.',
      ],
    },
    {
      id: 'limits',
      heading: '3. Technical limits',
      body: [
        'You may not intentionally evade recipient, attachment, quota, rate, warm-up or abuse controls. Automated activity must stay within published limits and must not degrade service for others.',
      ],
    },
    {
      id: 'enforcement',
      heading: '4. Enforcement',
      body: [
        'We may throttle, quarantine, suspend or terminate activity when evidence indicates abuse, compromise or material risk. Actions are recorded through the service audit workflow where applicable.',
      ],
    },
    {
      id: 'reporting',
      heading: '5. Reporting',
      body: [`Send abuse reports and supporting evidence to ${abuseEmail}.`],
    },
  ],
}

const security: LegalDoc = {
  id: 'security',
  title: 'Security',
  updated,
  intro:
    'This page summarizes security controls implemented by the CS Mail application. It intentionally avoids claiming certifications, audit reports, cloud providers or encryption properties that have not been independently established for a specific deployment.',
  sections: [
    {
      id: 'account',
      heading: '1. Account security',
      body: [
        'Passwords are hashed before storage. Refresh credentials are kept in HttpOnly cookies, session rotation detects replay, optional two-factor authentication is supported, and administrators can revoke sessions or suspend accounts.',
      ],
    },
    {
      id: 'application',
      heading: '2. Application controls',
      body: [
        'Server authorization is applied to mailbox, contacts, calendar, billing and administration APIs. Security-sensitive actions are audited. Authentication and public-abuse limits are shared across application replicas.',
        'The API validates trusted proxy boundaries, restricts CORS, emits defensive browser headers and separates liveness from dependency readiness for safer deployments.',
      ],
    },
    {
      id: 'operations',
      heading: '3. Operational security',
      body: [
        'Production deployment guidance keeps database, mail-management, monitoring and API origin ports private to the host or internal network and places public traffic behind a TLS reverse proxy.',
        'Backups cover PostgreSQL state and staged attachment bytes, include integrity hashes, and are paired with a non-destructive restore drill. Operators are responsible for protecting backup storage and testing recovery regularly.',
      ],
    },
    {
      id: 'monitoring',
      heading: '4. Monitoring and incident handling',
      body: [
        'The application exposes health and Prometheus-compatible metrics for deployment and monitoring systems. Customer-visible incidents can be published through the Status workflow.',
      ],
    },
    {
      id: 'contact',
      heading: '5. Security reports',
      body: [`Security or vulnerability reports can be sent to ${securityEmail}. Please do not include secrets in an initial report.`],
    },
  ],
}

const abuse: LegalDoc = {
  id: 'abuse',
  title: 'Abuse & Copyright Reports',
  updated,
  intro:
    'Use this process to report spam, phishing, malware, account compromise or copyright concerns involving CS Mail.',
  sections: [
    {
      id: 'report',
      heading: '1. What to send',
      body: [
        'Include the sender address, relevant message headers or message identifier, date and time, a description of the concern, and a contact address where we can follow up. Preserve the original message where practical.',
        'For a copyright notice, identify the protected work and allegedly infringing material, provide your contact details, and include the statements and signature required by the law applicable to your notice.',
      ],
    },
    {
      id: 'channel',
      heading: '2. Reporting channel',
      body: [`Send reports to ${abuseEmail}. Use a clear subject such as “Spam report”, “Phishing report” or “Copyright notice”.`],
    },
    {
      id: 'response',
      heading: '3. Review and action',
      body: [
        'Reports are reviewed against available evidence and applicable policy. Depending on the circumstances, action may include throttling, quarantine, account restriction, preservation of relevant evidence or other proportionate measures.',
      ],
    },
    {
      id: 'privacy',
      heading: '4. Account deletion',
      body: [`Users can initiate account deletion from Settings. Additional privacy requests can be sent to ${privacyEmail}.`],
    },
  ],
}

export const legalDocs: Record<string, LegalDoc> = { terms, privacy, aup, security, abuse }

export const legalNav: { id: string; label: string }[] = [
  { id: 'terms', label: 'Terms of Service' },
  { id: 'privacy', label: 'Privacy Policy' },
  { id: 'aup', label: 'Acceptable Use' },
  { id: 'security', label: 'Security' },
  { id: 'abuse', label: 'Abuse & Copyright' },
]
