export type Invoice = {
  id: string
  period: string
  date: string
  amount: string
  status: 'paid' | 'pending'
  plan: string
  seats: number
  rate: string
  subtotal: string
  tax: string
  total: string
  paymentMethod: string
  billedTo: string
}

export const invoices: Invoice[] = [
  {
    id: 'INV-2026-036',
    period: 'September 2026',
    date: 'September 1, 2026',
    amount: '$8.00',
    status: 'paid',
    plan: 'Harbor Team',
    seats: 1,
    rate: '$8.00',
    subtotal: '$8.00',
    tax: '$0.00',
    total: '$8.00',
    paymentMethod: 'Visa ending in 4049',
    billedTo: 'alex@harbor.co',
  },
  {
    id: 'INV-2026-033',
    period: 'August 2026',
    date: 'August 1, 2026',
    amount: '$8.00',
    status: 'paid',
    plan: 'Harbor Team',
    seats: 1,
    rate: '$8.00',
    subtotal: '$8.00',
    tax: '$0.00',
    total: '$8.00',
    paymentMethod: 'Visa ending in 4049',
    billedTo: 'alex@harbor.co',
  },
  {
    id: 'INV-2026-030',
    period: 'July 2026',
    date: 'July 1, 2026',
    amount: '$8.00',
    status: 'paid',
    plan: 'Harbor Team',
    seats: 1,
    rate: '$8.00',
    subtotal: '$8.00',
    tax: '$0.00',
    total: '$8.00',
    paymentMethod: 'Visa ending in 4049',
    billedTo: 'alex@harbor.co',
  },
  {
    id: 'INV-2026-027',
    period: 'June 2026',
    date: 'June 1, 2026',
    amount: '$8.00',
    status: 'paid',
    plan: 'Harbor Team',
    seats: 1,
    rate: '$8.00',
    subtotal: '$8.00',
    tax: '$0.00',
    total: '$8.00',
    paymentMethod: 'Visa ending in 4049',
    billedTo: 'alex@harbor.co',
  },
]

type InvoiceResolver = (id: string) => Invoice | undefined
let dynamicResolver: InvoiceResolver | null = null

/** Let the billing service supply invoices that live on the API as they load. */
export const registerInvoiceResolver = (resolver: InvoiceResolver) => {
  dynamicResolver = resolver
}

export const invoiceFor = (id: string): Invoice | undefined =>
  invoices.find((invoice) => invoice.id.toLowerCase() === id.toLowerCase()) ?? dynamicResolver?.(id)

export const invoiceToText = (invoice: Invoice) =>
  [
    'Harbor Mail receipt',
    '==================',
    `Invoice: ${invoice.id}`,
    `Period: ${invoice.period}`,
    `Date: ${invoice.date}`,
    `Status: ${invoice.status === 'paid' ? 'Paid' : 'Pending'}`,
    `Billed to: ${invoice.billedTo}`,
    `Payment: ${invoice.paymentMethod}`,
    '',
    `${invoice.plan} (${invoice.seats} seat)`,
    `  ${invoice.rate} x ${invoice.seats}`,
    `Subtotal: ${invoice.subtotal}`,
    `Tax: ${invoice.tax}`,
    `Total: ${invoice.total}`,
  ].join('\n')
