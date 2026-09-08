import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import http from 'node:http'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const __dirname = path.dirname(fileURLToPath(import.meta.url))
const root = path.resolve(__dirname, '..')
const distDir = path.join(root, 'dist')
const seedFile = path.join(root, 'src', 'data', 'messages.json')
const storeDir = path.join(__dirname, 'data')
const storeFile = path.join(storeDir, 'mailbox.json')
const uploadsDir = path.join(storeDir, 'uploads')
const port = Number(process.env.PORT || 8000)

const seed = existsSync(seedFile) ? readFileSync(seedFile, 'utf8') : '[]'

if (!existsSync(storeFile)) {
  mkdirSync(storeDir, { recursive: true })
  writeFileSync(storeFile, seed)
}

const readStore = () => {
  try { return readFileSync(storeFile, 'utf8') } catch { return seed }
}

const writeStore = data => {
  writeFileSync(storeFile, JSON.stringify(data, null, 2))
  return JSON.stringify(data)
}

const json = (response, status, body) => {
  const payload = JSON.stringify(body)
  response.writeHead(status, { 'content-type': 'application/json; charset=utf-8' })
  response.end(payload)
}

const readBody = request =>
  new Promise((resolve, reject) => {
    const chunks = []
    let size = 0
    request.on('data', chunk => {
      size += chunk.length
      if (size > 8 * 1024 * 1024) { reject(new Error('Payload too large')); request.destroy(); return }
      chunks.push(chunk)
    })
    request.on('end', () => resolve(Buffer.concat(chunks).toString('utf8')))
    request.on('error', reject)
  })

const readRawBody = request =>
  new Promise((resolve, reject) => {
    const chunks = []
    let size = 0
    request.on('data', chunk => {
      size += chunk.length
      if (size > 8 * 1024 * 1024) { reject(new Error('Payload too large')); request.destroy(); return }
      chunks.push(chunk)
    })
    request.on('end', () => resolve(Buffer.concat(chunks)))
    request.on('error', reject)
  })

const parseMultipart = (body, boundary) => {
  const parts = []
  const marker = Buffer.from(`--${boundary}`)
  let cursor = body.indexOf(marker)
  while (cursor !== -1) {
    const next = body.indexOf(marker, cursor + marker.length)
    if (next === -1) break
    const section = body.subarray(cursor + marker.length, next)
    const headerEnd = section.indexOf('\r\n\r\n')
    if (headerEnd !== -1) {
      const headers = section.subarray(0, headerEnd).toString('utf8')
      let fieldName = ''
      let filename = ''
      for (const line of headers.split('\r\n')) {
        const match = line.match(/name="([^"]*)"(?:;\s*filename="([^"]*)")?/)
        if (match) { fieldName = match[1]; filename = match[2] || '' }
      }
      let data = section.subarray(headerEnd + 4)
      if (data.length >= 2 && data[data.length - 2] === 0x0d && data[data.length - 1] === 0x0a) data = data.subarray(0, data.length - 2)
      parts.push({ fieldName, filename, data })
    }
    cursor = next
  }
  return parts
}

const mime = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.mjs': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.json': 'application/json; charset=utf-8',
  '.svg': 'image/svg+xml',
  '.ico': 'image/x-icon',
  '.png': 'image/png',
  '.webmanifest': 'application/manifest+json; charset=utf-8',
  '.woff2': 'font/woff2',
  '.txt': 'text/plain; charset=utf-8',
}

const serveStatic = (request, response) => {
  let pathname = decodeURIComponent(new URL(request.url, 'http://localhost').pathname)
  if (pathname.endsWith('/')) pathname += 'index.html'
  let file = path.normalize(path.join(distDir, pathname))
  if (!file.startsWith(distDir)) { response.writeHead(403); response.end('Forbidden'); return }

  if (!existsSync(file)) {
    if (request.method === 'GET' && !path.extname(pathname)) file = path.join(distDir, 'index.html')
    if (path.extname(pathname)) { response.writeHead(404); response.end('Not found'); return }
    if (!existsSync(file)) { response.writeHead(404); response.end('Not found'); return }
  }

  const type = mime[path.extname(file)] || 'application/octet-stream'
  const cacheControl = pathname.startsWith('/assets/') ? 'public, max-age=31536000, immutable' : 'no-cache'
  const body = readFileSync(file)
  response.writeHead(200, { 'content-type': type, 'cache-control': cacheControl, 'content-length': body.length })
  response.end(request.method === 'HEAD' ? undefined : body)
}

const server = http.createServer(async (request, response) => {
  const url = new URL(request.url, 'http://localhost')
  const { pathname } = url
  const method = request.method || 'GET'

  try {
    if (pathname.startsWith('/api/')) {
      if (method === 'POST' && pathname === '/api/auth/login') {
        let payload
        try { payload = JSON.parse(await readBody(request)) } catch { json(response, 400, { error: 'Invalid request body' }); return }
        const { email, password } = payload
        if (!email?.includes('@') || typeof password !== 'string' || password.length < 6) {
          json(response, 401, { error: 'Invalid email or password' })
          return
        }
        const session = Buffer.from(email).toString('base64url')
        response.writeHead(200, {
          'content-type': 'application/json; charset=utf-8',
          'set-cookie': `harbor-session=${session}; Path=/; HttpOnly; SameSite=Lax; Max-Age=${60 * 60 * 24 * 30}`,
        })
        response.end(JSON.stringify({ user: { email }, demo: true }))
        return
      }

      if (method === 'POST' && pathname === '/api/auth/logout') {
        response.writeHead(200, {
          'content-type': 'application/json; charset=utf-8',
          'set-cookie': 'harbor-session=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0',
        })
        response.end(JSON.stringify({ ok: true }))
        return
      }

      if (method === 'GET' && pathname === '/api/mail') { json(response, 200, JSON.parse(readStore())); return }
      if (method === 'PUT' && pathname === '/api/mail') {
        let payload
        try { payload = JSON.parse(await readBody(request)) } catch { json(response, 400, { error: 'Invalid request body' }); return }
        if (!Array.isArray(payload)) { json(response, 400, { error: 'Mailbox must be an array' }); return }
        json(response, 200, JSON.parse(writeStore(payload)))
        return
      }

      const attachmentMatch = pathname.match(/^\/api\/attachments\/([\w.\- ]+)$/)
      if (method === 'POST' && pathname === '/api/attachments') {
        const contentType = request.headers['content-type'] || ''
        const boundary = contentType.match(/boundary=(?:"([^"]+)"|([^;]+))/i)?.[1] ?? (contentType.match(/boundary=(?:"([^"]+)"|([^;]+))/i)?.[2])
        if (!boundary) { json(response, 400, { error: 'Expected multipart/form-data' }); return }
        const file = parseMultipart(await readRawBody(request), boundary).find(part => part.filename)
        if (!file || !file.data.length) { json(response, 400, { error: 'No file provided' }); return }
        mkdirSync(uploadsDir, { recursive: true })
        const safeName = path.basename(file.filename).replace(/[^\w.\- ]/g, '_')
        const stored = `${Date.now()}-${safeName}`
        writeFileSync(path.join(uploadsDir, stored), file.data)
        json(response, 200, { name: file.filename, size: file.data.length, url: `/api/attachments/${encodeURIComponent(stored)}` })
        return
      }
      if (method === 'GET' && attachmentMatch) {
        const file = path.normalize(path.join(uploadsDir, attachmentMatch[1]))
        if (!file.startsWith(uploadsDir) || !existsSync(file)) { json(response, 404, { error: 'Not found' }); return }
        const body = readFileSync(file)
        response.writeHead(200, { 'content-type': 'application/octet-stream', 'content-disposition': `attachment; filename="attachment"`, 'content-length': body.length })
        response.end(body)
        return
      }

      json(response, 404, { error: 'Not found' })
      return
    }

    if (method === 'GET' || method === 'HEAD') { serveStatic(request, response); return }
    response.writeHead(405, { 'allow': 'GET, HEAD' })
    response.end('Method not allowed')
  } catch (error) {
    console.error(error)
    if (!response.headersSent) json(response, 500, { error: 'Internal server error' })
  }
})

server.listen(port, () => {
  console.log(`Harbor Mail demo running at http://localhost:${port}`)
  console.log(`  - static site: ./dist`)
  console.log(`  - data store : ${storeFile}`)
  console.log(`Build the app first with "npm run build" if no site is served.`)
})