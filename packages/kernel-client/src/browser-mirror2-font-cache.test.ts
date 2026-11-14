// MIRROR-P2: a font kept for a later document stays bounded and an advertised digest never changes or leaves.
import test from 'node:test'
import assert from 'node:assert/strict'
import { Mirror2FontCache } from './browser-mirror2-font-cache.js'
const id = (n: number) => n.toString(16).padStart(24, '0').padEnd(64, '0')
const font = (bytes: number) => new Blob([new Uint8Array(bytes)], { type: 'font/woff2' })
test('MIRROR-P2 font retention is named by the first 24 hex digits, immutable and bounded in count', () => {
  const cache = new Mirror2FontCache()
  for (let n = 0; n < 64; n++) cache.remember(id(n), font(10))
  const advertised = cache.digests()
  assert.equal(advertised.length, 64)
  cache.remember(id(65), font(10))
  assert.deepEqual(cache.digests(), advertised)
  const first = cache.take(id(0))
  cache.remember(id(0), font(20))
  assert.equal(cache.take(id(0)), first, 'a retained definition is immutable')
  cache.clear(); assert.deepEqual(cache.digests(), [])
})
test('MIRROR-P2 font bytes bound the retained set independently of its count', () => {
  const cache = new Mirror2FontCache()
  for (let n = 0; n < 5; n++) cache.remember(id(n), font(4 * 1024 * 1024))
  assert.equal(cache.digests().length, 4, '16 MiB fit four 4 MiB fonts')
})
