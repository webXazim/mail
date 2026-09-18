import { readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'

const root = process.cwd()
const rel = (p) => join(root, p)

/* Wells confirmed earlier via Select-String: next line pairs ink-2 bg with
 * white text. Both stay dark in light mode -> invisible text. We repaint them
 * with the flipping pair (panel-soft surface / paper text) used everywhere else
 * in light. Dark mode changes 0: panel-soft #0e1413 ≈ ink-2 #12181c, paper
 * #f2f1eb = white #f2f1eb. */

const DARK_BG = 'background: var(--a2t-ink-2);'
const DARK_TX = 'color: var(--a2t-white);'
const LIGHT_BG = 'background: var(--a2t-panel-soft);'
const LIGHT_TX = 'color: var(--a2t-paper);'

const swaps = [
  { file: 'src/styles/admin.css', bg: 306, tx: 307 },
  { file: 'src/styles/admin.css', bg: 347, tx: 348 },
  { file: 'src/styles/auth.css', bg: 63, tx: 64 },
  { file: 'src/styles/composer.css', bg: 294, tx: 295 },
  { file: 'src/styles/composer.css', bg: 335, tx: 336 },
  { file: 'src/styles/settings.css', bg: 94, tx: 95 },
  { file: 'src/styles/settings.css', bg: 258, tx: 259 },
  { file: 'src/styles/settings.css', bg: 292, tx: 293 },
  { file: 'src/styles/settings.css', bg: 358, tx: 359 },
  { file: 'src/styles/settings.css', bg: 395, tx: 396 },
]

let ok = 0
for (const s of swaps) {
  const p = rel(s.file)
  const lines = readFileSync(p, 'utf8').split('\n')
  const bg = lines[s.bg - 1]
  const tx = lines[s.tx - 1]
  if (bg.trim() !== DARK_BG || tx.trim() !== DARK_TX) {
    console.log(`SKIP ${s.file}:${s.bg} got [${bg.trim()}] + [${tx.trim()}]`)
    continue
  }
  lines[s.bg - 1] = bg.replace(DARK_BG, LIGHT_BG)
  lines[s.tx - 1] = tx.replace(DARK_TX, LIGHT_TX)
  writeFileSync(p, lines.join('\n'))
  console.log(`OK  ${s.file}:${s.bg} ${s.tx} -> panel-soft + paper`)
  ok++
}
console.log(`wells repainted: ${ok}/${swaps.length}`)
