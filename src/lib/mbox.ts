import type { Mail } from '../types'

const safeColors = new Set(['coral', 'teal', 'purple', 'orange', 'blue', 'green'])

const escapeFromLines = (body: string) => body.split('\n').map(line => line.startsWith('From ') ? `>${line}` : line).join('\n')

const headerBlock = (mail: Mail): string => {
  const headerLines = [
    `From alex@harbor.co ${mail.time}`,
    `From: ${mail.sender} <${mail.email}>`,
    mail.to && mail.to.length ? `To: ${mail.to.join(', ')}` : '',
    mail.cc && mail.cc.length ? `Cc: ${mail.cc.join(', ')}` : '',
    `Subject: ${mail.subject}`,
    `Date: ${mail.time}`,
    `X-Harbor-Id: ${mail.id}`,
    `X-Harbor-Initials: ${mail.initials}`,
    `X-Harbor-Sender: ${mail.sender}`,
    `X-Harbor-Sender-Email: ${mail.email}`,
    `X-Harbor-Color: ${mail.color}`,
    `X-Harbor-Folder: ${mail.folder || 'Inbox'}`,
    `X-Harbor-Label: ${mail.label}`,
    `X-Harbor-Unread: ${mail.unread ? 'yes' : 'no'}`,
    `X-Harbor-Starred: ${mail.starred ? 'yes' : 'no'}`,
    `X-Harbor-Attachment: ${mail.attachment ? 'yes' : 'no'}`,
    mail.attachmentName ? `X-Harbor-Attachment-Name: ${mail.attachmentName}` : '',
    mail.snoozedUntil ? `X-Harbor-Snoozed-Until: ${mail.snoozedUntil}` : '',
  ]
  return `${headerLines.filter(line => line !== '').join('\n')}\n\n${escapeFromLines(mail.preview)}`
}

export const exportToMbox = (mails: Mail[]): string => mails.map(headerBlock).join('\n\n')

const readHeader = (headers: string, header: string): string => {
  const line = headers.split('\n').find(candidate => candidate.toLowerCase().startsWith(`${header.toLowerCase()}:`))
  return line ? line.slice(line.indexOf(':') + 1).trim() : ''
}

export const importFromMbox = (content: string): Mail[] => {
  const blocks = content.split(/\n(?=From )/).map(block => block.trim().replace(/^From [^\n]*\n/, ''))
  const mails: Mail[] = []
  for (const block of blocks) {
    const [headers = '', ...bodyParts] = block.split('\n\n')
    const id = readHeader(headers, 'X-Harbor-Id')
    if (!id) continue
    const toRaw = readHeader(headers, 'To')
    const ccRaw = readHeader(headers, 'Cc')
    const color = readHeader(headers, 'X-Harbor-Color')
    mails.push({
      id: id.startsWith('imported-') ? id : `imported-${id}`,
      initials: readHeader(headers, 'X-Harbor-Initials') || '?',
      sender: readHeader(headers, 'X-Harbor-Sender') || readHeader(headers, 'From'),
      email: readHeader(headers, 'X-Harbor-Sender-Email'),
      subject: readHeader(headers, 'Subject') || '(no subject)',
      preview: bodyParts.join('\n').replace(/^>/gm, '') || '(no body)',
      time: readHeader(headers, 'Date') || 'Just now',
      label: readHeader(headers, 'X-Harbor-Label') || 'Updates',
      color: safeColors.has(color) ? color : 'teal',
      unread: readHeader(headers, 'X-Harbor-Unread') === 'yes',
      starred: readHeader(headers, 'X-Harbor-Starred') === 'yes',
      folder: readHeader(headers, 'X-Harbor-Folder') || 'Inbox',
      attachment: readHeader(headers, 'X-Harbor-Attachment') === 'yes',
      attachmentName: readHeader(headers, 'X-Harbor-Attachment-Name') || undefined,
      snoozedUntil: readHeader(headers, 'X-Harbor-Snoozed-Until') || undefined,
      to: toRaw ? toRaw.split(',').map(part => part.trim()).filter(Boolean) : undefined,
      cc: ccRaw ? ccRaw.split(',').map(part => part.trim()).filter(Boolean) : undefined,
    })
  }
  return mails
}