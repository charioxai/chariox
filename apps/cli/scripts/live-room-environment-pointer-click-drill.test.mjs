import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import test from 'node:test'

const script = await readFile(new URL('./live-room-environment-pointer-click-drill.mjs', import.meta.url), 'utf8')

test('MP-08/MP-10/MP-11 A06: the slice-bound agent cannot open sudo or generate a Vault password', () => {
  assert.doesNotMatch(script, /mcpToolCall\(providerRun, "create_generated_credential", \{\n/, 'the removed ordinary generator must not be expected to succeed')
  assert.match(script, /"\/sudo [^"]*",\s*\[\],\s*\)\),\s*\/sudo requires a local regular agent\//, 'the owner /sudo for a slice-bound agent is refused')
  assert.match(script, /\["chariox\.vault\.generate", \{ request_id:/, 'the worker refuses Vault generation outside a sudo window')
  assert.match(script, /assert\.equal\(secretActions\.length, 1\)/, 'only the user-entered credential is typed')
})
