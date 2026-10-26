// MP-11 F7: public protocol admission never requires private MCP configuration.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { waitForProviderRunMcpAdmission } from '../apps/cli/scripts/lib/provider-run-mcp-admission.mjs'

test('MP-11 protocol-435 native MCP admission matches the owning agent grant', async () => {
  const run = { id: 'run', state: 'Running', session_id: 'session', agent_instance_id: 'owner' }
  const client = { send: async request => request.GetProviderRun ? { ProviderRun: { provider_run: run } }
    : { AgentsListed: { agents: [{ id: 'owner', extension_grants: [{ kind: 'mcp', name: 'echo' }] }] } } }
  assert.equal(await waitForProviderRunMcpAdmission(client, 'run', 'echo', 20), run)
})

test('MP-11 another agent grant and a starting run cannot prove admission', async () => {
  for (const [state, agentId] of [['Running', 'foreign'], ['Starting', 'owner']]) {
    const client = { send: async request => request.GetProviderRun ? { ProviderRun: { provider_run: {
      id: 'run', state, session_id: 'session', agent_instance_id: 'owner',
    } } } : { AgentsListed: { agents: [{ id: agentId, extension_grants: [{ kind: 'mcp', name: 'echo' }] }] } } }
    await assert.rejects(waitForProviderRunMcpAdmission(client, 'run', 'echo', 10), /timed out/)
  }
})
