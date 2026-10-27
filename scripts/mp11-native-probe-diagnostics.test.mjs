import assert from 'node:assert/strict'
import test from 'node:test'
import { summarizeEnvironment, summarizeMcpStatus } from '../experiments/mcp-isolation-spike/src/opencode-native-config-acceptance.mjs'

test('MP-11 F31 environment evidence records only allowlisted presence, lengths and config counts', () => {
  const sentinel = 'synthetic_db_private_value'
  const environment = { DATABASE_URL: `postgres://u:${sentinel}@db/test`,
    OPENCODE_CONFIG: `https://u:${sentinel}@config/test`,
    OPENCODE_CONFIG_CONTENT: JSON.stringify({ mcp: { [sentinel]: { environment: { DATABASE_URL: sentinel } } }, plugin: [`https://u:${sentinel}@plugin/test`] }),
    PATH: '/usr/bin', HOME: '/synthetic/profile' }
  const summary = summarizeEnvironment(environment)
  assert.equal(JSON.stringify(summary).includes(sentinel), false)
  assert.equal('DATABASE_URL' in summary, false)
  assert.deepEqual(summary.PATH, { present: true, length: 8 })
})

test('MP-11 F31 native MCP failure evidence excludes arbitrary error tails', () => {
  const sentinel = 'synthetic_connection_private_value'
  const summary = summarizeMcpStatus({ fixture: { status: 'failed', error: `postgres://u:${sentinel}@db/test`, tools: [] } })
  assert.equal(JSON.stringify(summary).includes(sentinel), false)
})
