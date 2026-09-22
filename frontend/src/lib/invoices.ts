export type Invoice = {
  id: string
  period: string
  date: string
  dueDate: string
  amount: string
  status: 'issued' | 'paid' | 'void'
  plan: string
  mailboxCount: number
  includedMailboxCount: number
  extraMailboxCount: number
  extraMailboxUnitPrice: string
  extraMailboxTotal: string
  baseRate: string
  rate: string
  subtotal: string
  tax: string
  taxRate: string
  total: string
  paymentMethod: string
  billedTo: string
  sellerName: string
  sellerVat: string
  sellerCr: string
  sellerAddress: string
  buyerName: string
  buyerVat: string
  buyerCr: string
  buyerAddress: string
}

type InvoiceResolver = (id: string) => Invoice | undefined
let dynamicResolver: InvoiceResolver | null = null

export const registerInvoiceResolver = (resolver: InvoiceResolver) => {
  dynamicResolver = resolver
}

export const invoiceFor = (id: string): Invoice | undefined => dynamicResolver?.(id)

export const invoiceToText = (invoice: Invoice) =>
  [
    'CS Mail invoice',
    '==================',
    `Invoice: ${invoice.id}`,
    `Issue date: ${invoice.date}`,
    `Due date: ${invoice.dueDate}`,
    `Status: ${invoice.status.toUpperCase()}`,
    '',
    `Seller: ${invoice.sellerName}`,
    invoice.sellerVat ? `Seller VAT: ${invoice.sellerVat}` : '',
    invoice.sellerCr ? `Seller CR: ${invoice.sellerCr}` : '',
    invoice.sellerAddress,
    '',
    `Billed to: ${invoice.buyerName || invoice.billedTo}`,
    invoice.buyerVat ? `Buyer VAT: ${invoice.buyerVat}` : '',
    invoice.buyerCr ? `Buyer CR: ${invoice.buyerCr}` : '',
    invoice.buyerAddress,
    invoice.billedTo,
    '',
    `${invoice.plan} (${invoice.includedMailboxCount} included mailbox${invoice.includedMailboxCount === 1 ? '' : 'es'}; ${invoice.mailboxCount} total)`,
    `Base plan: ${invoice.baseRate}`,
    invoice.extraMailboxCount > 0 ? `${invoice.extraMailboxCount} additional mailbox${invoice.extraMailboxCount === 1 ? '' : 'es'} × ${invoice.extraMailboxUnitPrice}` : '',
    `Subtotal: ${invoice.subtotal}`,
    `Tax (${invoice.taxRate}): ${invoice.tax}`,
    `Total: ${invoice.total}`,
    `Payment method: ${invoice.paymentMethod}`,
  ]
    .filter(Boolean)
    .join('\n')
