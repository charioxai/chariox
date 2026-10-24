// MP-08 / MP-11: companion requirements follow the shared owner-Copy minimums.
import assert from 'node:assert/strict'
import test from 'node:test'
import {readFile} from 'node:fs/promises'
test('MP-08 / MP-11 companion contract requires the current owner-Copy protocol pair', async () => {
  const root = new URL('../../../', import.meta.url)
  const contract = await readFile(new URL('BYOM_CLOUD_CONTRACT.md', root), 'utf8')
  const shared = await readFile(new URL('packages/kernel-client/src/ipc-managed-context-requests.ts', root), 'utf8')
  const local = shared.match(/OWNER_MANAGED_CONTEXT_MINIMUM_PROTOCOL_VERSION = (\d+)/)[1]
  const peer = shared.match(/OWNER_MANAGED_CONTEXT_MINIMUM_RELAY_PROTOCOL_VERSION = (\d+)/)[1]
  assert.ok(contract.includes(`protocol **${local}**`))
  assert.ok(contract.includes(`relay peer **${peer}**`))
})
