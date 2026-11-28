import assert from 'node:assert/strict'
import { test } from 'node:test'
import { makeRepo } from '../lib/check-fixture.mjs'
import check from './agent-tools-on-demand.mjs'

const LIST = 'apps/kernel/agent_upfront_tools.json'
const LOADER = 'apps/kernel/src/transport/mcp_server/tool_loading.rs'
const lists = agent => `${JSON.stringify({ claude: { agent }, opencode: { agent: ['chariox.agents.run'] } }, null, 2)}\n`
const base = () => ({ [LIST]: lists(['chariox.agents.run', 'chariox.events.inbox']), [LOADER]: 'include_str!("../../../agent_upfront_tools.json")\n' })

test('a shrunk list and an unchanged tree pass', () => {
  const r = makeRepo(base())
  assert.equal(r.run(check.name).status, 0)
  r.write(LIST, lists(['chariox.agents.run']))
  r.commit('shrink')
  assert.equal(r.run(check.name).status, 0)
  r.dispose()
})

test('a tool added to an upfront list is red and its raise line allows only that list', () => {
  const r = makeRepo(base())
  r.write(LIST, lists(['chariox.agents.run', 'chariox.events.inbox', 'chariox.vault.use_env']))
  r.commit('load vault upfront')
  const red = r.run(check.name)
  assert.equal(red.status, 1)
  assert.match(red.text, /agent_upfront_tools\.claude\.agent gains chariox\.vault\.use_env without a "budget raised: agent_upfront_tools\.claude\.agent: <why>" line/)
  r.git('commit', '-q', '--allow-empty', '-m', 'raise\n\nbudget raised: agent_upfront_tools.opencode.agent: wrong list')
  assert.equal(r.run(check.name).status, 1)
  r.git('commit', '-q', '--allow-empty', '-m', 'raise\n\nbudget raised: agent_upfront_tools.claude.agent: vault is needed by every run')
  assert.equal(r.run(check.name).status, 0)
  r.dispose()
})

test('"*" where the base had a list is red; an always-load key elsewhere is red; a loader that ignores the list is red', () => {
  const r = makeRepo(base())
  r.write(LIST, `${JSON.stringify({ claude: { agent: ['chariox.agents.run', 'chariox.events.inbox'] }, opencode: { agent: '*' } })}\n`)
  r.write('apps/kernel/src/transport/other.rs', 'const K: &str = "anthropic/alwaysLoad";\n')
  r.write(LOADER, 'const X: u8 = 1;\n')
  r.commit('everything upfront')
  const red = r.run(check.name)
  assert.equal(red.status, 1)
  assert.match(red.text, /opencode\.agent is raised from a list to "\*"/)
  assert.match(red.text, /other\.rs:1: marks a tool load-upfront outside tool_loading\.rs/)
  assert.match(red.text, /tool_loading\.rs:0: does not read/)
  r.dispose()
})
