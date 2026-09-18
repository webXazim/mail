import { readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'

const files = [
  'src/styles/admin.css',
  'src/styles/auth.css',
  'src/styles/composer.css',
  'src/styles/settings.css',
]

const DARK_BG = 'background: var(--a2t-ink-2);'
const DARK_TX = 'color: var(--a2t-white);'
const LIGHT_BG = 'background: var(--a2t-panel-soft);'
const LIGHT_TX = 'color: var(--a2t-paper);'

const dry = process.argv.includes('--dry')

let total = 0
for (const rel of files) {
  const p = join('src/styles', rel.replace('src/styles/', ''))
  const text = readFileSync(p, 'utf8')
  const lines = text.split('\n')
  let swaps = 0
  for (let i = 0; i < lines.length; i++) {
    if (lines[i].includes(DARK_BG) && i + 1 < lines.length && lines[i + 1].includes(DARK_TX)) {
      lines[i] = lines[i].replace(DARK_BG, LIGHT_BG)
      lines[i + 1] = lines[i + 1].replace(DARK_TX, LIGHT_TX)
      swaps++
    }
  }
  if (swaps) {
    total += swaps
    console.log(`${rel.includes('admin') ? 'admin' : rel.split('/')[1] || rel}: ${swaps} well(s)`)
    if (!dry) writeFileSync(p, lines.join('\n'), 'utf8')
  }
}
console.log(`--- total swaps: ${total}${dry ? ' (dry-run, files untouched)' : ''}`)
