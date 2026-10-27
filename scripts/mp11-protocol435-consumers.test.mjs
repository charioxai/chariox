// MP-11 #879: live consumers must work with the credential-free protocol-435 DTO.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { readFileSync } from 'node:fs'
import { startHostedRemoteProviderRun } from '../apps/cli/scripts/lib/hosted-cloud-kernel-scenarios.mjs'

test('MP-11 hosted remote consumer accepts a protocol-435 provider-run response', async () => {
  const requests = {
    submitPromptRequest: () => ({ SubmitPrompt: {} }),
    listAgentsRequest: () => ({ ListAgents: {} }),
    getProviderRunRequest: () => ({ GetProviderRun: {} }),
  }
  const publicRun = { id: 'leased:lease:worker', session_id: 'session', provider: 'dev-stub',
    model: 'runtime-mcp-fixture', state: 'Running' }
  const client = { send: async request => request.ListAgents
    ? { AgentsListed: { agents: [{ id: 'agent', remote_execution: { leased_agent_id: 'lease', active_worker_provider_run_id: 'worker' } }] } }
    : request.GetProviderRun ? { ProviderRun: { provider_run: publicRun } } : { PromptSubmitted: {} } }
  const result = await startHostedRemoteProviderRun({ client, requests, sessionId: 'session', attachmentId: 'attachment',
    agentId: 'agent', prompt: 'initialize', timeoutMs: 30, pollMs: 1 })
  assert.equal(result.id, publicRun.id)
  assert.equal(result.runtime_mcp_auth_token, undefined)
})

const consumers = [
  'apps/cli/scripts/lib/hosted-cloud-kernel-scenarios.mjs',
  'apps/cli/scripts/lib/native-tui-capabilities.mjs',
  'apps/cli/scripts/lib/live-provider-thread-transfer-local-worker-scenarios.mjs',
  'apps/cli/scripts/lib/live-provider-thread-transfer-runtime.mjs',
  'apps/cli/scripts/live-remote-home-extension-drill.mjs',
  'apps/cli/scripts/live-room-environment-pointer-click-drill.mjs',
  'apps/cli/scripts/live-room-repetition-drill.mjs',
  'apps/cli/scripts/live-runtime-register-extension-drill.mjs',
  'apps/cli/scripts/live-secret-handoff-drill.mjs',
  'apps/cli/scripts/live-metaagent-code-fix-drill.mjs',
  'apps/cli/scripts/live-metaagent-task-lifecycle-drill.mjs',
  'apps/cli/scripts/live-metaagent-provider-capability-import-drill.mjs',
]
for (const file of consumers) test(`MP-11 ${file} no longer reads credentials from public runs`, () => {
  const source = readFileSync(new URL(`../${file}`, import.meta.url), 'utf8')
  assert.doesNotMatch(source, /\.(?:runtime_mcp_server_url|runtime_mcp_auth_token|mcp_servers|resume_state)\b/)
})
