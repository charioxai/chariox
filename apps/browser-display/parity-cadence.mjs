// MP-08 / MP-10: paired arms use the same uniformly jittered input gaps.
import { createHash } from 'node:crypto'

export function parityCadence(repeat, site, kind, mode = 'jitter') {
  if (![1, 2, 3].includes(repeat) || !['click', 'type'].includes(kind) || !['fixed', 'jitter'].includes(mode)) {
    throw new Error('MP-10: invalid paired cadence')
  }
  const label = `r15:${repeat}:${site}:${kind}`
  let state = createHash('sha256').update(label).digest().readUInt32LE(0)
  const gaps = []
  return {
    next() {
      state = (state + 0x6D2B79F5) >>> 0
      let value = state
      value = Math.imul(value ^ (value >>> 15), value | 1)
      value ^= value + Math.imul(value ^ (value >>> 7), value | 61)
      const gap = mode === 'fixed' ? (kind === 'click' ? 1400 : 1000) : 1200 + 500 * (((value ^ (value >>> 14)) >>> 0) / 4294967296)
      gaps.push(gap)
      return gap
    },
    record() { return { mode, seed_label: label, gaps: [...gaps] } },
  }
}
