import { getProviderRunRequest, getSessionStateRequest } from '@arroba/kernel-client'
import { compactWorkflowRunSummary } from './workflow-code-artifact-drill-topology.mjs'
import { sleep, stage, unwrap } from './workflow-code-artifact-drill-runtime.mjs'

export async function waitForCompletedWorkflowRun(client, sessionId, workflowRunId, timeoutMs, progressLabel = null) {
  const deadline = Date.now() + timeoutMs
  let lastRun = null
  let lastProgressAt = 0
  let lastProgressKey = null
  while (Date.now() < deadline) {
    const stateResponse = await withTimeout(
      client.send(getSessionStateRequest(sessionId)),
      Math.min(30_000, timeoutMs),
      `${progressLabel ?? 'workflow run'} status poll timed out for ${workflowRunId}`,
    )
    const state = unwrap(stateResponse, 'SessionStateLoaded')?.session
      ?? unwrap(stateResponse, 'SessionState')?.session
    const run = (state?.workflow_runs ?? []).find((entry) => entry.id === workflowRunId)
    if (run) {
      lastRun = run
      if (progressLabel) {
        const nodeStatuses = (run.node_runs ?? [])
          .map((nodeRun) => `${nodeRun.node_id}:${nodeRun.status}`)
          .join(',')
        const progressKey = `${run.status}|${nodeStatuses}|${run.messages?.length ?? 0}|${run.intermediate_outputs?.length ?? 0}`
        const now = Date.now()
        if (progressKey !== lastProgressKey || now - lastProgressAt >= 10_000) {
          stage(`${progressLabel}: workflow run status`, {
            workflow_run_id: workflowRunId,
            status: run.status,
            node_runs: (run.node_runs ?? []).map((nodeRun) => ({
              node_id: nodeRun.node_id,
              status: nodeRun.status,
              failures: nodeRun.failures?.length ?? 0,
            })),
            messages: run.messages?.length ?? 0,
            intermediate_outputs: run.intermediate_outputs?.length ?? 0,
          })
          lastProgressKey = progressKey
          lastProgressAt = now
        }
      }
      if (['Completed', 'Failed', 'Stopped'].includes(run.status)) {
        return run
      }
    }
    await sleep(500)
  }
  throw new Error(`workflow run ${workflowRunId} did not complete before timeout${lastRun ? `\n${JSON.stringify(compactWorkflowRunSummary(lastRun), null, 2)}` : ''}`)
}

export async function waitForProviderRunReady(client, providerRunId, timeoutMs) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const providerRun = unwrap(await client.send(getProviderRunRequest(providerRunId)), 'ProviderRun')?.provider_run
    if (providerRun?.state && providerRun.state !== 'Starting') {
      if (providerRun.state !== 'Running' && providerRun.state !== 'Parked') {
        throw new Error(`provider run ${providerRunId} reached unexpected state ${providerRun.state}`)
      }
      return providerRun
    }
    await sleep(250)
  }
  throw new Error(`provider run ${providerRunId} did not become ready`)
}
