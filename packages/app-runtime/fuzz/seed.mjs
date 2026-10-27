// Writes the wire_decode seed corpus from the SDK wire vectors. Each seed is
// the target's selector byte (bit 0: 0 worker, 1 supervisor) and the frame.
// Usage: node seed.mjs && cargo +nightly fuzz run wire_decode
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs'

const vectors = JSON.parse(readFileSync(new URL('../../app-sdk/test/wire-vectors.json', import.meta.url), 'utf8'))
const corpus = new URL('corpus/wire_decode/', import.meta.url)
mkdirSync(corpus, { recursive: true })
const cases = [...vectors.cases.map((vector) => ({ ...vector, json: JSON.stringify(vector.message) })), ...vectors.rawCases]
for (const { name, sender, json } of cases) {
  writeFileSync(new URL(name, corpus), Buffer.concat([Buffer.from([sender === 'worker' ? 0 : 1]), Buffer.from(json)]))
}
console.log(`${cases.length} seeds in ${corpus.pathname}`)
