import { readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'

const FILES = [
  ['admin.css', 306, 307],
  ['admin.css', 347, 348],
  ['auth.css', 63, 64],
  ['composer.css', 294, 295],
  ['composer.css', 335, 336],
  ['settings.css', 94, 95],
  ['settings.css', 258, 259],
  ['settings.css', 292, 293],
  ['settings.css', 358, 359],
  ['settings.css', 395, 396],
]
const ROOT = process.cwd()
let total = 0
for (const [f, bg, tx] of FILES) {
  const p = join(ROOT, 'src', 'styles', f)
  const lines = readFileSync(p, 'utf8').split('\n')
  const bbg = (lines[bg - 1] || '').trim()
  const btx = (lines[tx - 1] || '').trim()
  if (bbg !== 'background: var(--a2t-ink-2);' || btx !== 'color: var(--a2t-white);') {
    console.log(`SKIP ${f}:${bg} got [${bbg}][${btx}]`)
    continue
  }
  lines[bg - 1] = lines[bg - 1].replace('var(--a2t-ink-2)', 'var(--a2t-panel-soft)')
  lines[tx - 1] = lines[tx - 1].replace('var(--a2t-white)', 'var(--a2t-paper)')
  writeFileSync(p, lines.join('\n'))
  total++
  console.log(`OK  ${f}:${bg}+${tx} -> panel-soft + paper`)
}
console.log(`wells repainted: ${total}/10`)
