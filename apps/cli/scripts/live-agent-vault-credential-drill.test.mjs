import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import test from 'node:test'

const script = await readFile(new URL('./live-agent-vault-credential-drill.mjs', import.meta.url), 'utf8')

test('vault history leak scan requires the provider prompt to become idle first', () => {
  const idleHelper = script.match(/const idle = \(timeoutMs\) => waitForAgentPromptIdle\(\{[\s\S]*?\}\)(?:\.catch\(\(\) => \{\}\))?/)
  assert.ok(idleHelper, 'vault drill must wait for its provider prompt to become idle')
  assert.doesNotMatch(idleHelper[0], /\.catch\(/, 'idle timeout or lookup failure must fail the drill')

  const lastIdle = script.lastIndexOf('await idle(')
  const transcriptSnapshot = script.indexOf('const transcript = await providerTranscript')
  assert.ok(lastIdle > idleHelper.index, 'vault drill must await the idle helper')
  assert.ok(transcriptSnapshot > lastIdle, 'provider history must be snapshotted only after the last idle wait')
})

test('MP-08/MP-10/MP-11 A06: generation runs only in an owner-authorized sudo window', () => {
  assert.doesNotMatch(script, /create_generated_credential`\) with|call `chariox\.create_generated_credential`/, 'the removed ordinary generator must not be called')
  const ordinary = script.indexOf('M17_NO_GENERATION_TOOL')
  const sudo = script.indexOf("'/sudo ")
  const generate = script.indexOf('Call `chariox.vault.generate` exactly once')
  assert.ok(ordinary > 0 && sudo > ordinary && generate > sudo, 'ordinary refusal precedes the /sudo generation prompt')
  assert.match(script, /respondToSudoPopup\(\{[\s\S]*?passkey: vaultPassphrase/, 'the owner answers the sudo popup with the passkey')
  assert.match(script, /waitForSudoWindowsToEnd\(/, 'the sudo window must end with its work')
})


test('MP-08/MP-10/MP-11 A06: the live drill validates the saved canonical origin', () => {
  const condition = script.match(/if \((generated\.length !== 1 \|\|[\s\S]*?)\) \{\n\s+throw new Error\(`\$\{provider\} sudo generation/)
  assert.ok(condition, 'exercise the actual live-drill credential validation guard')
  const refuses = new Function('generated', 'credential', 'origin', 'echo', `return (${condition[1]})`)
  const credential = {
    id: 'generated-handle', metadata: { created_by_kind: 'vault_generate' },
    source: { key: 'generated-handle' }, allowed_uses: ['browser'],
  }
  const origin = 'http://127.0.0.1:8123/register'
  for (const [saved, expected] of [
    ['http://127.0.0.1:8123', false],
    ['127.0.0.1:8123', true],
    ['https://127.0.0.1:8123', true],
    ['http://127.0.0.1:8124', true],
  ]) {
    const stored = { ...credential, allowed_hosts: [saved] }
    assert.equal(refuses([stored], stored, origin, { port: 8123 }), expected, `saved origin ${saved}`)
  }
})
