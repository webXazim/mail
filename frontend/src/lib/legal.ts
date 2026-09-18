export type LegalSection = { id: string; heading: string; body: string[] }

export type LegalDoc = {
  id: string
  title: string
  updated: string
  intro: string
  sections: LegalSection[]
}

const terms: LegalDoc = {
  id: 'terms',
  title: 'Terms of Service',
  updated: '1 September 2026',
  intro:
    'These Terms of Service ("Terms") govern your access to and use of the Harbor Mail platform and related services (collectively, the "Services") provided by Harbor Mail, Inc. ("Harbor," "we," or "us"). By creating an account or using the Services, you agree to be bound by these Terms.',
  sections: [
    {
      id: 'eligibility',
      heading: '1. Eligibility',
      body: [
        'You may use the Services only if you are at least 18 years old (or the age of majority in your jurisdiction) and capable of forming a binding contract. You represent that all registration information you submit is truthful, accurate, and that you will keep it current.',
        'If you are using the Services on behalf of an organization, you represent that you have the authority to bind that organization to these Terms.',
      ],
    },
    {
      id: 'accounts',
      heading: '2. Accounts',
      body: [
        'You are responsible for maintaining the confidentiality of your account credentials and for all activity that occurs under your account. Notify Harbor immediately of any unauthorized use.',
        'We reserve the right to suspend or terminate accounts that are being used in violation of these Terms or that pose a security risk to the platform.',
      ],
    },
    {
      id: 'acceptable-use',
      heading: '3. Acceptable Use',
      body: [
        'You agree not to use the Services to send spam, distribute malware, engage in phishing, harass others, or violate any applicable law. Detailed requirements are set out in our Acceptable Use Policy, which is incorporated into these Terms by reference.',
      ],
    },
    {
      id: 'privacy',
      heading: '4. Privacy',
      body: [
        'Our collection and use of personal information is governed by our Privacy Policy. By using the Services, you consent to the processing practices described therein.',
      ],
    },
    {
      id: 'content',
      heading: '5. Your Content',
      body: [
        'You retain ownership of all data, messages, files, and other content you submit through the Services ("Your Content"). You grant Harbor a limited license to host, transmit, and display Your Content solely for the purpose of providing the Services to you.',
        'You are solely responsible for the legality of Your Content. Harbor does not pre-screen content but reserves the right to remove material that violates these Terms.',
      ],
    },
    {
      id: 'service-level',
      heading: '6. Service Level',
      body: [
        'Harbor Mail targets 99.9% monthly uptime for production services. Scheduled maintenance windows are communicated in advance. Service level commitments and remedies are detailed in the Service Level Agreement available at /legal/sla.',
      ],
    },
    {
      id: 'fees',
      heading: '7. Fees and Billing',
      body: [
        'Fees for paid plans are quoted in USD and billed in advance on a monthly or annual cycle depending on your selection. Fees are non-refundable except as required by law or as expressly stated in these Terms.',
        "Harbor reserves the right to change pricing with 30 days' notice. Changes take effect at the start of the next billing cycle after notice.",
      ],
    },
    {
      id: 'termination',
      heading: '8. Termination',
      body: [
        'You may cancel your account at any time from Settings. Upon termination, your right to use the Services ceases immediately. Harbor will make your data available for export for 30 days following termination, after which it will be deleted.',
      ],
    },
    {
      id: 'warranty',
      heading: '9. Warranty Disclaimer',
      body: [
        'THE SERVICES ARE PROVIDED "AS IS" WITHOUT WARRANTIES OF ANY KIND, EITHER EXPRESS OR IMPLIED. HARBOR DISCLAIMS ALL WARRANTIES, INCLUDING BUT NOT LIMITED TO IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE.',
      ],
    },
    {
      id: 'liability',
      heading: '10. Limitation of Liability',
      body: [
        "TO THE MAXIMUM EXTENT PERMITTED BY LAW, HARBOR SHALL NOT BE LIABLE FOR ANY INDIRECT, INCIDENTAL, SPECIAL, CONSEQUENTIAL, OR PUNITIVE DAMAGES, OR ANY LOSS OF PROFITS OR REVENUES, WHETHER INCURRED DIRECTLY OR INDIRECTLY. HARBOR'S TOTAL LIABILITY FOR ANY CLAIMS ARISING FROM THESE TERMS SHALL NOT EXCEED THE AMOUNT YOU PAID TO HARBOR IN THE 12 MONTHS IMMEDIATELY PRECEDING THE EVENT GIVING RISE TO THE CLAIM.",
      ],
    },
    {
      id: 'changes',
      heading: '11. Changes to These Terms',
      body: [
        'We may update these Terms from time to time. Material changes will be communicated via email or in-app notice at least 30 days before taking effect. Continued use of the Services after the effective date constitutes acceptance of the updated Terms.',
      ],
    },
    {
      id: 'contact',
      heading: '12. Contact',
      body: [
        'Questions about these Terms should be directed to legal@harbor.co or Harbor Mail, Inc., 1 Harbor Way, San Francisco, CA 94111.',
      ],
    },
  ],
}

const privacy: LegalDoc = {
  id: 'privacy',
  title: 'Privacy Policy',
  updated: '1 September 2026',
  intro:
    'This Privacy Policy describes how Harbor Mail, Inc. ("Harbor," "we," or "us") collects, uses, and protects your personal information when you use the Harbor Mail platform and related services (the "Services").',
  sections: [
    {
      id: 'information-collected',
      heading: '1. Information We Collect',
      body: [
        'Account information: name, email address, password (hashed), and billing details when you create an account.',
        'Message content: emails, attachments, and metadata (sender, recipients, subject, timestamps) transmitted through the Services.',
        'Usage data: interaction logs, device information, browser type, IP address, and access times collected automatically.',
        'Payment information: processed by our third-party payment processor and not stored on Harbor servers.',
      ],
    },
    {
      id: 'use',
      heading: '2. How We Use Your Information',
      body: [
        'To provide, maintain, and improve the Services, including message delivery, storage, search, and synchronization.',
        'To detect and prevent security threats, fraud, and abuse of the platform.',
        'To send service-related communications such as security alerts, policy updates, and billing notices.',
        'To comply with legal obligations and enforce our terms.',
      ],
    },
    {
      id: 'sharing',
      heading: '3. Sharing and Disclosure',
      body: [
        'We do not sell your personal information. We share information only with service providers who assist in operating the platform (infrastructure, payment processing, analytics) and only under contractual obligations that protect your data.',
        'We may disclose information when required by law, valid legal process, or to protect the rights and safety of Harbor and its users.',
      ],
    },
    {
      id: 'security',
      heading: '4. Data Security',
      body: [
        'All data is encrypted in transit (TLS 1.2+) and at rest (AES-256). We maintain SOC 2 Type II compliance and undergo annual third-party security audits.',
        'Access to production systems is restricted to authorized personnel via role-based access controls and requires multi-factor authentication.',
      ],
    },
    {
      id: 'retention',
      heading: '5. Data Retention',
      body: [
        'Your account data is retained for as long as your account is active. Upon account deletion, data is purged within 30 days, except where retention is required by law or necessary for legitimate business purposes.',
        'Backup data is rotated on a 90-day cycle and encrypted at rest.',
      ],
    },
    {
      id: 'rights',
      heading: '6. Your Rights',
      body: [
        'Depending on your jurisdiction, you may have the right to access, correct, port, or delete your personal information. You can exercise most of these rights directly from Settings.',
        'For requests not covered by Settings, contact privacy@harbor.co. We will respond within 30 days.',
      ],
    },
    {
      id: 'cookies',
      heading: '7. Cookies and Tracking',
      body: [
        'Harbor Mail uses strictly necessary cookies for authentication and session management. We do not use advertising or third-party tracking cookies.',
        'Analytics data is collected in aggregate and does not identify individual users.',
      ],
    },
    {
      id: 'transfers',
      heading: '8. International Transfers',
      body: [
        'Data is processed in data centers located in the United States and the European Economic Area. Transfers outside the EEA are governed by Standard Contractual Clauses (SCCs) approved by the European Commission.',
      ],
    },
    {
      id: 'children',
      heading: "9. Children's Privacy",
      body: [
        'The Services are not directed to children under 16. We do not knowingly collect personal information from children.',
      ],
    },
    {
      id: 'changes',
      heading: '10. Changes to This Policy',
      body: [
        'We may update this Privacy Policy from time to time. Material changes will be communicated at least 30 days before taking effect via email or in-app notice.',
      ],
    },
    {
      id: 'contact',
      heading: '11. Contact',
      body: [
        'For questions about this Privacy Policy or to exercise your data rights, contact our Data Protection Officer at privacy@harbor.co or Harbor Mail, Inc., 1 Harbor Way, San Francisco, CA 94111.',
      ],
    },
  ],
}

const aup: LegalDoc = {
  id: 'aup',
  title: 'Acceptable Use Policy',
  updated: '1 September 2026',
  intro:
    'This Acceptable Use Policy ("AUP") sets out the standards of conduct required when using the Harbor Mail platform. It supplements and is incorporated into the Terms of Service.',
  sections: [
    {
      id: 'prohibited-content',
      heading: '1. Prohibited Content',
      body: [
        'You may not use the Services to send, store, or distribute content that is illegal, harmful, threatening, abusive, harassing, defamatory, or that infringes the intellectual property rights of any third party.',
        'You may not distribute malware, phishing content, or links to malicious sites.',
      ],
    },
    {
      id: 'spam',
      heading: '2. Spam and Unsolicited Mail',
      body: [
        'Harbor has a zero-tolerance spam policy. You may not send unsolicited bulk messages, purchased mailing lists, or messages that generate excessive complaints. Violations result in immediate account suspension.',
        'Transactional and marketing emails must comply with CAN-SPAM, CASL, and GDPR requirements including opt-out mechanisms and sender identification.',
      ],
    },
    {
      id: 'security',
      heading: '3. Security',
      body: [
        'You may not attempt to gain unauthorized access to the Services, other accounts, or connected systems. You may not probe, scan, or test the vulnerability of the platform without written authorization.',
        'You must not use the Services to send phishing messages or distribute credentials-harvesting content.',
      ],
    },
    {
      id: 'network',
      heading: '4. Network Abuse',
      body: [
        'You may not use the Services to conduct denial-of-service attacks, relay spam through open mail servers, or engage in any form of network abuse.',
      ],
    },
    {
      id: 'enforcement',
      heading: '5. Enforcement',
      body: [
        'Harbor monitors platform activity for violations. We may suspend or terminate access, remove content, and report violations to law enforcement authorities without notice.',
        'We will investigate credible reports of AUP violations and take appropriate action within 24 hours.',
      ],
    },
    {
      id: 'reporting',
      heading: '6. Reporting Violations',
      body: [
        'Report suspected violations to abuse@harbor.co with supporting evidence. Reports are reviewed within one business day.',
      ],
    },
  ],
}

const security: LegalDoc = {
  id: 'security',
  title: 'Security & Compliance',
  updated: '1 September 2026',
  intro:
    'Harbor Mail is built from the ground up to protect the confidentiality, integrity, and availability of your communications. This page describes our security practices and compliance posture.',
  sections: [
    {
      id: 'encryption',
      heading: '1. Encryption',
      body: [
        'All data in transit is protected by TLS 1.2 or higher. Data at rest is encrypted using AES-256 with keys managed through AWS KMS.',
        'Message content is encrypted end-to-end in transit between clients using TLS mutual authentication.',
      ],
    },
    {
      id: 'infrastructure',
      heading: '2. Infrastructure',
      body: [
        'Harbor Mail runs on AWS in isolated VPCs across multiple availability zones. Infrastructure is provisioned using Terraform and managed through immutable deployment pipelines.',
        'Production systems are scanned continuously for vulnerabilities and patched within 48 hours of critical CVE disclosure.',
      ],
    },
    {
      id: 'access',
      heading: '3. Access Controls',
      body: [
        'Access to production systems requires multi-factor authentication, role-based access controls, and VPN with IP allowlisting.',
        'All access is logged, auditable, and reviewed quarterly. No engineer has standing root access to production databases.',
      ],
    },
    {
      id: 'compliance',
      heading: '4. Compliance',
      body: [
        'Harbor Mail maintains SOC 2 Type II certification with annual third-party audits covering security, availability, and confidentiality trust services criteria.',
        'GDPR compliance is maintained through our Data Processing Agreement (DPA), available upon request.',
      ],
    },
    {
      id: 'incident',
      heading: '5. Incident Response',
      body: [
        'Harbor maintains a documented incident response plan with defined escalation procedures. Critical incidents are triaged within 15 minutes, with status updates published to status.harbor.co.',
        'Affected users are notified within 72 hours of a confirmed breach, consistent with GDPR Article 33.',
      ],
    },
    {
      id: 'penetration',
      heading: '6. Penetration Testing',
      body: [
        'Harbor engages independent third-party firms to conduct annual penetration tests and quarterly vulnerability assessments. Summary findings are available to enterprise customers under NDA.',
      ],
    },
    {
      id: 'data-processing',
      heading: '7. Data Processing Agreement',
      body: [
        'Our DPA governs the processing of personal data on behalf of customers and includes Standard Contractual Clauses for international transfers. Contact legal@harbor.co to request a copy.',
      ],
    },
    {
      id: 'status',
      heading: '8. Status and Uptime',
      body: [
        'Real-time platform status is published at status.harbor.co. Our SLA guarantees 99.9% monthly uptime with service credits for qualifying downtime.',
      ],
    },
    {
      id: 'contact',
      heading: '9. Contact',
      body: [
        'For security inquiries, vulnerability reports, or DPA requests, contact security@harbor.co or visit harbor.co/security.',
      ],
    },
  ],
}

export const legalDocs: Record<string, LegalDoc> = { terms, privacy, aup, security }

export const legalNav: { id: string; label: string }[] = [
  { id: 'terms', label: 'Terms of Service' },
  { id: 'privacy', label: 'Privacy Policy' },
  { id: 'aup', label: 'Acceptable Use' },
  { id: 'security', label: 'Security' },
]
