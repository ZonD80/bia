// Every language must have every text English has, with the same {{placeholders}} and <0>marks</0>;
// run by deploy.sh before the build (the build itself stops at a key missing for a page).
import { readdirSync, readFileSync } from 'node:fs'
import { join } from 'node:path'

const dir = join(import.meta.dirname, 'app/locales')
const flat = (o, p = '') => Object.entries(o).flatMap(([k, v]) => (v && typeof v === 'object' ? flat(v, `${p}${k}.`) : [[p + k, String(v)]]))
const marks = (s) => (s.match(/\{\{\w+\}\}|<\/?\d+\/?>/g) || []).sort().join(' ')
const read = (lng, file) => Object.fromEntries(flat(JSON.parse(readFileSync(join(dir, lng, file), 'utf8'))))
const langs = readdirSync(dir).filter((l) => l !== 'en')
let bad = 0
for (const file of readdirSync(join(dir, 'en'))) {
  const en = read('en', file)
  for (const lng of langs) {
    let other
    try {
      other = read(lng, file)
    } catch {
      console.error(`${lng}/${file}: missing`)
      bad++
      continue
    }
    for (const [k, v] of Object.entries(en)) {
      const base = k.replace(/_(one|other)$/, '')
      const t = other[k] ?? (k !== base ? other[`${base}_other`] : undefined)
      if (t === undefined) (console.error(`${lng}/${file}: no ${k}`), bad++)
      else if (marks(t) !== marks(v)) (console.error(`${lng}/${file}: ${k} has "${marks(t)}", English "${marks(v)}"`), bad++)
    }
    for (const k of Object.keys(other)) if (!(k in en) && !/_(zero|one|two|few|many|other)$/.test(k)) console.warn(`${lng}/${file}: unused ${k}`)
  }
}
if (bad) {
  console.error(`${bad} translation problem(s)`)
  process.exit(1)
}
console.log(`translations complete: ${langs.join(', ')}`)
