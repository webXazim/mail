export type HelpCategory = 'Getting started' | 'Mail & inbox' | 'Security' | 'Billing'

export type HelpArticle = { category: HelpCategory; title: string; summary: string }

export const helpArticles: HelpArticle[] = [
  {
    category: 'Getting started',
    title: 'Switching from another inbox',
    summary:
      'Import your contacts, connect your old address, and set your layout in the first ten minutes.',
  },
  {
    category: 'Getting started',
    title: 'Keyboard shortcuts',
    summary:
      'j/k to move, Enter to open, e to archive, # to delete, s to star — press ? in the app anytime.',
  },
  {
    category: 'Getting started',
    title: 'Unified inbox & accounts',
    summary: 'Combine personal and work addresses into one calm view, or keep them separate.',
  },
  {
    category: 'Mail & inbox',
    title: 'Filters & routing',
    summary: 'Build rules that sort, label, forward, or snooze messages the moment they arrive.',
  },
  {
    category: 'Mail & inbox',
    title: 'Labels, folders & the archive',
    summary: 'File mail without moving it: cross-labels, custom folders, and a true archive.',
  },
  {
    category: 'Mail & inbox',
    title: 'Sending on a schedule',
    summary: 'Draft tonight, send at 9am — scheduled sends respect every recipient timezone.',
  },
  {
    category: 'Security',
    title: 'Two-factor authentication',
    summary: 'Add a second factor to your login and approve sessions from trusted devices.',
  },
  {
    category: 'Security',
    title: 'Spam & phishing defense',
    summary: 'How our filters, sender scoring, and quarantine keep the noise out.',
  },
  {
    category: 'Security',
    title: 'Your audit log',
    summary: 'Every sign-in, password change, and billing event in one accountable history.',
  },
  {
    category: 'Billing',
    title: 'Plans, seats & switching',
    summary: 'Understand Solo, Team, and Business — and what happens when you change.',
  },
  {
    category: 'Billing',
    title: 'Invoices & receipts',
    summary: 'Find past invoices, view receipts, and download copies for your records.',
  },
  {
    category: 'Billing',
    title: 'Payment methods',
    summary: 'Add, remove, or set a default card, and keep billing current.',
  },
]

export type Faq = { q: string; a: string }

export const faqs: Faq[] = [
  {
    q: 'Can I use my own domain?',
    a: 'Yes. Add your domain in the Admin center, verify the DNS records, and mail flows to your CS Mail mailboxes.',
  },
  {
    q: 'How long does email stay on your servers?',
    a: 'Your mail is kept until you delete it. After cancellation we keep data for 30 days, then remove it.',
  },
  {
    q: 'Is my end-to-end traffic encrypted?',
    a: 'All traffic is TLS-encrypted in transit, and messages are encrypted at rest with per-mailbox keys.',
  },
  {
    q: 'Can I cancel at any time?',
    a: 'Yes — plans are monthly with no contracts. You can cancel from Billing and keep access until the period ends.',
  },
  {
    q: 'Do you read my mail for ads?',
    a: 'Never. CS Mail is not an advertising product. Your content is used only to deliver, search, and route your mail.',
  },
]

export const filterHelp = (query: string) => {
  const q = query.trim().toLowerCase()
  if (!q) return { articles: helpArticles, faqs }
  return {
    articles: helpArticles.filter((article) =>
      `${article.title} ${article.summary} ${article.category}`.toLowerCase().includes(q),
    ),
    faqs: faqs.filter((faq) => `${faq.q} ${faq.a}`.toLowerCase().includes(q)),
  }
}
