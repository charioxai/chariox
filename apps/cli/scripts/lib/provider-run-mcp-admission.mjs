// MP-11 F7: public admission evidence; this does not claim tool execution.
import { getProviderRunRequest, listAgentsRequest } from '@chariox/kernel-client'
import { setTimeout as sleep } from 'node:timers/promises'

export async function waitForProviderRunMcpAdmission(client, providerRunId, mcpName, timeoutMs = 90000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const run = (await client.send(getProviderRunRequest(providerRunId))).ProviderRun?.provider_run
    if (run?.state === 'Running' && run.agent_instance_id) {
      const agents = (await client.send(listAgentsRequest(run.session_id))).AgentsListed?.agents ?? []
      const agent = agents.find(candidate => candidate.id === run.agent_instance_id)
      if (agent?.extension_grants?.some(grant => grant.kind === 'mcp' && grant.name === mcpName)) return run
    }
    await sleep(Math.min(500, Math.max(0, deadline - Date.now())))
  }
  throw Error(`timed out waiting for public agent MCP admission for run ${providerRunId}`)
}
