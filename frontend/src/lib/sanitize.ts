import DOMPurify from 'dompurify'

const ALLOWED_TAGS = [
  'a', 'abbr', 'b', 'blockquote', 'br', 'code', 'del', 'div', 'em', 'h1', 'h2',
  'h3', 'h4', 'h5', 'h6', 'hr', 'i', 'img', 'li', 'ol', 'p', 'pre', 's',
  'strong', 'sub', 'sup', 'table', 'tbody', 'td', 'th', 'thead', 'tr', 'u',
  'ul',
]

const ALLOWED_ATTR = ['href', 'rel', 'target', 'title', 'alt', 'src', 'width', 'height']

DOMPurify.addHook('afterSanitizeAttributes', (node) => {
  if (node.tagName === 'A') {
    node.setAttribute('target', '_blank')
    node.setAttribute('rel', 'noopener noreferrer')
  }
  if (node.tagName === 'IMG') {
    const src = node.getAttribute('src')?.trim().toLowerCase() ?? ''
    if (src.startsWith('data:') && !src.startsWith('data:image/')) {
      node.removeAttribute('src')
    }
  }
})

export function sanitizeHtml(html: string): string {
  return DOMPurify.sanitize(html, {
    ALLOWED_TAGS,
    ALLOWED_ATTR,
    FORBID_TAGS: ['iframe', 'object', 'embed', 'link', 'style', 'form'],
    FORBID_ATTR: ['style', 'onclick', 'onload', 'onerror', 'onmouseover'],
    USE_PROFILES: { html: true },
    ADD_ATTR: [],
  })
}
