import type { Mail } from '../types'

const safeColors = new Set(['coral', 'teal', 'purple', 'orange', 'blue', 'green'])

const escapeFromLines = (body: string) =>
  body
    .split('\n')
    .map((line) => (line.startsWith('From ') ? `>${line}` : line))
    .join('\n')

const headerBlock = (mail: Mail): string => {
  const headerLines = [
    `From alex@crescentsphere.com ${mail.time}`,
    `From: ${mail.sender} <${mail.email}>`,
    mail.to && mail.to.length ? `To: ${mail.to.join(', ')}` : '',
    mail.cc && mail.cc.length ? `Cc: ${mail.cc.join(', ')}` : '',
    `Subject: ${mail.subject}`,
    `Date: ${mail.time}`,
    `X-CS-Mail-Id: ${mail.id}`,
    `X-CS-Mail-Initials: ${mail.initials}`,
    `X-CS-Mail-Sender: ${mail.sender}`,
    `X-CS-Mail-Sender-Email: ${mail.email}`,
    `X-CS-Mail-Color: ${mail.color}`,
    `X-CS-Mail-Folder: ${mail.folder || 'Inbox'}`,
    `X-CS-Mail-Label: ${mail.label}`,
    `X-CS-Mail-Unread: ${mail.unread ? 'yes' : 'no'}`,
    `X-CS-Mail-Starred: ${mail.starred ? 'yes' : 'no'}`,
    `X-CS-Mail-Attachment: ${mail.attachment ? 'yes' : 'no'}`,
    mail.attachmentName ? `X-CS-Mail-Attachment-Name: ${mail.attachmentName}` : '',
    mail.snoozedUntil ? `X-CS-Mail-Snoozed-Until: ${mail.snoozedUntil}` : '',
  ]
  return `${headerLines.filter((line) => line !== '').join('\n')}\n\n${escapeFromLines(mail.preview)}`
}

export const exportToMbox = (mails: Mail[]): string => mails.map(headerBlock).join('\n\n')

const readHeader = (headers: string, header: string): string => {
  const line = headers
    .split('\n')
    .find((candidate) => candidate.toLowerCase().startsWith(`${header.toLowerCase()}:`))
  return line ? line.slice(line.indexOf(':') + 1).trim() : ''
}

const readProductHeader = (headers: string, suffix: string): string =>
  readHeader(headers, `X-CS-Mail-${suffix}`) || readHeader(headers, `X-Harbor-${suffix}`)

export const importFromMbox = (content: string): Mail[] => {
  const blocks = content
    .split(/\n(?=From )/)
    .map((block) => block.trim().replace(/^From [^\n]*\n/, ''))
  const mails: Mail[] = []
  for (const block of blocks) {
    const [headers = '', ...bodyParts] = block.split('\n\n')
    const id = readProductHeader(headers, 'Id')
    if (!id) continue
    const toRaw = readHeader(headers, 'To')
    const ccRaw = readHeader(headers, 'Cc')
    const color = readProductHeader(headers, 'Color')
    mails.push({
      id: id.startsWith('imported-') ? id : `imported-${id}`,
      initials: readProductHeader(headers, 'Initials') || '?',
      sender: readProductHeader(headers, 'Sender') || readHeader(headers, 'From'),
      email: readProductHeader(headers, 'Sender-Email'),
      subject: readHeader(headers, 'Subject') || '(no subject)',
      preview: bodyParts.join('\n').replace(/^>/gm, '') || '(no body)',
      time: readHeader(headers, 'Date') || 'Just now',
      label: readProductHeader(headers, 'Label') || 'Updates',
      color: safeColors.has(color) ? color : 'teal',
      unread: readProductHeader(headers, 'Unread') === 'yes',
      starred: readProductHeader(headers, 'Starred') === 'yes',
      folder: readProductHeader(headers, 'Folder') || 'Inbox',
      attachment: readProductHeader(headers, 'Attachment') === 'yes',
      attachmentName: readProductHeader(headers, 'Attachment-Name') || undefined,
      snoozedUntil: readProductHeader(headers, 'Snoozed-Until') || undefined,
      to: toRaw
        ? toRaw
            .split(',')
            .map((part) => part.trim())
            .filter(Boolean)
        : undefined,
      cc: ccRaw
        ? ccRaw
            .split(',')
            .map((part) => part.trim())
            .filter(Boolean)
        : undefined,
    })
  }
  return mails
}
