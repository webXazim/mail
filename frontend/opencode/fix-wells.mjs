import { readFileSync, writeFileSync } from 'node:fs'

// 10 verified input-well pairs: [rel, bgLine(1-idx), colorLine(1-idx)]
// bg is always exactly "  background: var(--a2t-ink-2);"
// tx is always exactly "  color: var(--a2t-white);"   -> swap to flipping pair
const WELLS = [
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

const p = (f) => 'src/styles/' + f
const apply = process.argv.includes('--apply')
let ok = 0
for (const [file, bg, tx] of WELLS) {
  const lines = readFileSync(p(file), 'utf8').split('\n')
  const a = lines[bg - 1], b = lines[tx - 1]
  const look = a.includes('var(--a2t-ink-2)') && b.includes('var(--a2t-white)')
  if (!look) { console.log('SKIP ' + file + ':' + bg + ' (no exact pair)'); continue }
  for (let i = bg - 1; i <= tx - 1; i++) {
    lines[i] = lines[i]
      .replaceAll('var(--a2t-ink-2)', 'var(--a2t-panel-soft)')
      .replaceAll('var(--a2t-white)', 'var(--a2t-paper)')
  }
  if (apply) writeFileSync(p(file), lines.join('\n'))
  ok++
  console.log((apply ? 'REPAINTED ' : 'DRY       ') + file + ':' + bg + '->' + tx)
}
console.log(`wells repainted: ${ok}/10 (apply=${apply})`)
