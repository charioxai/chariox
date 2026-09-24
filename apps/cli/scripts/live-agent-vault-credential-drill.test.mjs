import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import test from 'node:test'

const script = await readFile(new URL('./live-agent-vault-credential-drill.mjs', import.meta.url), 'utf8')

test('vault history leak scan requires the provider prompt to become idle first', () => {
  const idleWait = script.match(/await waitForAgentPromptIdle\(\{[\s\S]*?\}\)(?:\.catch\(\(\) => \{\}\))?/)
  assert.ok(idleWait, 'vault drill must wait for its provider prompt to become idle')
  assert.doesNotMatch(idleWait[0], /\.catch\(/, 'idle timeout or lookup failure must fail the drill')

  const transcriptSnapshot = script.indexOf('const transcript = await providerTranscript')
  assert.ok(transcriptSnapshot > idleWait.index, 'provider history must be snapshotted only after the idle wait')
})
