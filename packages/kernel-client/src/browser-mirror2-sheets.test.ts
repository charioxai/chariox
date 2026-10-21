// MP-08/MP-10/MP-11: advertised digests remain valid through reordered credits.
import test from 'node:test'
import assert from 'node:assert/strict'
import { Mirror2SheetCache } from './browser-mirror2-sheets.js'
const digest = (n: number) => n.toString(16).padStart(24, '0')
test('MP-11 sheet retention stays bounded and never evicts an advertised digest', () => {
  const cache = new Mirror2SheetCache()
  for (let n = 0; n < 128; n++) cache.remember({[digest(n)]: 'p{color:red}'})
  const advertised = cache.digests()
  cache.remember({[digest(129)]: 'p{color:blue}'})
  assert.deepEqual(cache.digests(), advertised)
  cache.remember({[digest(0)]: 'changed'})
  assert.equal(cache.texts.get(digest(0)), 'p{color:red}', 'retained definitions are immutable')
  cache.clear(); assert.deepEqual(cache.digests(), [])
})
test('MP-11 UTF-8 bytes bound the retained dictionary independently of its count', () => {
  const cache = new Mirror2SheetCache(), text = 'é'.repeat(512 * 1024)
  for (let n = 0; n < 10; n++) cache.remember({[digest(n)]: text})
  assert.equal(cache.digests().length, 8)
  assert(cache.digests().every(d => cache.texts.get(d) === text))
})
