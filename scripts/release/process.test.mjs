import assert from 'node:assert/strict'
import test from 'node:test'
import { resourceGuard } from './process.mjs'

test('MP-07 / MP-11: reserved filesystem blocks do not weaken the 15 percent free floor', async () => {
  const dependencies = { statfsImpl: async () => ({ blocks: 1000, bfree: 200, bavail: 145 }),
    readFileImpl: async () => 'MemAvailable: 67108864 kB\n' }
  await assert.rejects(resourceGuard('/lane-owned-output', dependencies), /disk free below 15%/)
  dependencies.statfsImpl = async () => ({ blocks: 1000, bfree: 200, bavail: 150 })
  await resourceGuard('/lane-owned-output', dependencies)
  dependencies.readFileImpl = async () => 'MemAvailable: 1000000 kB\n'
  await assert.rejects(resourceGuard('/lane-owned-output', dependencies), /MemAvailable/)
})
