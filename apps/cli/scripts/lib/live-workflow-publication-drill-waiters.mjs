import net from 'node:net'
import path from 'node:path'
import { cp, readFile, readdir, rm, writeFile } from 'node:fs/promises'
const { LocalIpcClient } = await import('../../../../packages/kernel-client/dist/ipc.js')
const { getDaemonHealthRequest, getProviderRunRequest, getSessionStateRequest, getWorkflowPublicationRequest, getWorkflowRunRequest, listSessionsRequest, listWorkflowRunsRequest } = await import('../../../../packages/kernel-client/dist/ipc-requests.js')
import { isTerminalWorkflowRunStatus, variant } from './live-workflow-publication-drill-runtime.mjs'

export async function assertGatewayDoesNotListen(baseUrl, timeoutMs = 1_500) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    try {
      const response = await fetch(`${baseUrl}/health`)
      if (response.ok) {
        throw new Error(`gateway unexpectedly listened at ${baseUrl}`)
      }
    } catch (error) {
      if (/unexpectedly listened/.test(error.message)) throw error
    }
    await new Promise((resolve) => setTimeout(resolve, 150))
  }
}

export async function waitForKernel(kernelUrl) {
  const deadline = Date.now() + 20_000
  let lastError = null
  while (Date.now() < deadline) {
    const client = new LocalIpcClient(kernelUrl)
    try {
      await client.send(listSessionsRequest())
      await client.close().catch(() => {})
      return
    } catch (error) {
      lastError = error
      await client.close().catch(() => {})
      await new Promise((resolve) => setTimeout(resolve, 250))
    }
  }
  throw new Error(`kernel did not become ready: ${lastError?.message ?? String(lastError)}`)
}

export async function waitForGateway(baseUrl, timeoutMs = 20_000) {
  const deadline = Date.now() + timeoutMs
  let lastError = null
  while (Date.now() < deadline) {
    try {
      const response = await fetch(`${baseUrl}/health`)
      if (response.ok) return
      lastError = new Error(`health status ${response.status}`)
    } catch (error) {
      lastError = error
    }
    await new Promise((resolve) => setTimeout(resolve, 250))
  }
  throw new Error(`gateway did not become ready: ${lastError?.message ?? String(lastError)}`)
}

export async function waitForContainerGateway(baseUrl, containerProcess, timeoutMs = 60_000) {
  try {
    await waitForGateway(baseUrl, timeoutMs)
  } catch (error) {
    throw new Error([
      `container gateway did not become ready: ${error instanceof Error ? error.message : String(error)}`,
      `stdout:\n${containerProcess?.logs?.stdout ?? ''}`,
      `stderr:\n${containerProcess?.logs?.stderr ?? ''}`,
    ].join('\n'))
  }
}

export async function assertPublicationRuntimeSessionHidden(client, runtimeSessionId) {
  const session = variant(await client.send(getSessionStateRequest(runtimeSessionId)), 'SessionState').session
  if (session?.id !== runtimeSessionId || session.hidden !== true) {
    throw new Error(`expected publication runtime session ${runtimeSessionId} to be hidden, got ${JSON.stringify(session)}`)
  }
  const sessions = variant(await client.send(listSessionsRequest()), 'SessionsListed').sessions ?? []
  if (sessions.some((candidate) => candidate.id === runtimeSessionId)) {
    throw new Error(`publication runtime session ${runtimeSessionId} leaked into normal session list`)
  }
}

export async function assertPackageDoesNotContain(exportDir, forbiddenValues) {
  const forbidden = forbiddenValues.filter((value) => typeof value === 'string' && value.length > 0)
  async function visit(dir) {
    for (const entry of await readdir(dir, { withFileTypes: true })) {
      const entryPath = path.join(dir, entry.name)
      if (entry.isDirectory()) {
        await visit(entryPath)
        continue
      }
      if (!entry.isFile()) continue
      const contents = await readFile(entryPath, 'utf8').catch(() => null)
      if (contents == null) continue
      for (const value of forbidden) {
        if (contents.includes(value)) {
          throw new Error(`publication package file ${entryPath} contains forbidden runtime token material`)
        }
      }
    }
  }
  await visit(exportDir)
}

export async function createUnavailableProviderPackage(sourceDir, targetDir) {
  await rm(targetDir, { recursive: true, force: true })
  await cp(sourceDir, targetDir, { recursive: true })
  await rm(path.join(targetDir, 'bindings.local.json'), { force: true })
  const snapshotPath = path.join(targetDir, 'workflow.snapshot.json')
  const snapshot = JSON.parse(await readFile(snapshotPath, 'utf8'))
  const agent = snapshot.agents?.[0]
  if (!agent?.id) {
    throw new Error(`exported publication snapshot has no agent to mutate: ${JSON.stringify(snapshot.agents)}`)
  }
  agent.provider = 'missing-publication-provider'
  agent.model = 'missing-publication-model'
  agent.effort = 'missing-publication-effort'
  await writeFile(snapshotPath, `${JSON.stringify(snapshot, null, 2)}\n`)
  return { agentId: agent.id }
}

export async function waitForTcpPort(host, port, timeoutMs = 20_000) {
  const deadline = Date.now() + timeoutMs
  let lastError = null
  while (Date.now() < deadline) {
    try {
      await new Promise((resolve, reject) => {
        const socket = net.createConnection({ host, port }, () => {
          socket.end()
          resolve()
        })
        socket.once('error', reject)
      })
      return
    } catch (error) {
      lastError = error
      await new Promise((resolve) => setTimeout(resolve, 250))
    }
  }
  throw new Error(`tcp port ${host}:${port} did not become ready: ${lastError?.message ?? String(lastError)}`)
}

export async function waitForRelayTarget(relayUrl, relayToken, targetDaemonAlias) {
  const deadline = Date.now() + 20_000
  let lastError = null
  while (Date.now() < deadline) {
    const relayClient = new LocalIpcClient(relayUrl, {
      relayAuthToken: relayToken,
      targetDaemonAlias,
      kernelPingIntervalMs: 60_000,
      kernelMaxMissedPongs: 10,
    })
    try {
      await Promise.race([
        relayClient.send(listSessionsRequest()),
        new Promise((_, reject) => setTimeout(() => reject(new Error('relay probe timeout')), 2_000)),
      ])
      await relayClient.close().catch(() => {})
      return
    } catch (error) {
      lastError = error
      await relayClient.close().catch(() => {})
      await new Promise((resolve) => setTimeout(resolve, 250))
    }
  }
  throw new Error(`relay target ${targetDaemonAlias} did not become reachable: ${lastError?.message ?? String(lastError)}`)
}

export async function waitForRegisteredPublicationEndpoint(client, sessionId, publicationId, expectedLocalUrl, expectedOpenUrlPrefix) {
  const deadline = Date.now() + 20_000
  let lastPublication = null
  while (Date.now() < deadline) {
    const response = variant(
      await client.send(getWorkflowPublicationRequest(sessionId, publicationId)),
      'WorkflowPublication',
    )
    lastPublication = response.publication ?? null
    if (
      lastPublication?.status === 'running'
      && lastPublication.deployment?.local_url === expectedLocalUrl
      && typeof lastPublication.open_url === 'string'
      && lastPublication.open_url.startsWith(expectedOpenUrlPrefix)
    ) {
      return lastPublication
    }
    await new Promise((resolve) => setTimeout(resolve, 250))
  }
  throw new Error(`publication endpoint did not register as running at ${expectedLocalUrl}: ${JSON.stringify(lastPublication)}`)
}

export async function waitForProviderRunReady(client, providerRunId) {
  const deadline = Date.now() + 20_000
  while (Date.now() < deadline) {
    const providerRun = variant(await client.send(getProviderRunRequest(providerRunId)), 'ProviderRun').provider_run
    if (providerRun?.state && providerRun.state !== 'Starting') {
      if (providerRun.state !== 'Running' && providerRun.state !== 'Parked') {
        throw new Error(`provider run ${providerRunId} reached unexpected state ${providerRun.state}`)
      }
      return providerRun
    }
    await new Promise((resolve) => setTimeout(resolve, 250))
  }
  throw new Error(`provider run ${providerRunId} did not become ready`)
}

export async function waitForScheduledWorkflowRun(client, sessionId, workflowId, options = {}) {
  const deadline = Date.now() + (options.timeoutMs ?? (options.requireOutput ? 30_000 : 20_000))
  let lastRuns = []
  let lastRun = null
  while (Date.now() < deadline) {
    const listed = variant(
      await client.send(listWorkflowRunsRequest(sessionId, workflowId)),
      'WorkflowRunsListed',
    )
    lastRuns = listed.workflow_runs ?? []
    if (lastRuns.length > 0) {
      const candidate = lastRuns[0]
      const detailed = variant(
        await client.send(getWorkflowRunRequest(sessionId, candidate.id)),
        'WorkflowRun',
      ).workflow_run ?? candidate
      lastRun = detailed
      if (!options.requireOutput || (isTerminalWorkflowRunStatus(detailed.status) && detailed.final_output?.message)) {
        return detailed
      }
    }
    await new Promise((resolve) => setTimeout(resolve, 250))
  }
  let activeClaims = []
  try {
    const health = variant(await client.send(getDaemonHealthRequest()), 'DaemonHealth')
    activeClaims = health.projection?.workspace_coordination?.active_operation_claims ?? []
  } catch {
    activeClaims = []
  }
  throw new Error(`scheduled publication did not reach expected run state; active claims: ${JSON.stringify(activeClaims)}, last run: ${JSON.stringify(lastRun)}, last runs: ${JSON.stringify(lastRuns)}`)
}

export async function waitForPublicationStatusLatestOutput(gatewayUrl, expectedMessage) {
  const deadline = Date.now() + 30_000
  let lastStatus = null
  while (Date.now() < deadline) {
    const response = await fetch(`${gatewayUrl}/.well-known/arroba/publication/status`)
    lastStatus = await response.json()
    if (response.status === 200 && lastStatus.latest_output?.kind === 'final' && lastStatus.latest_output?.message === expectedMessage) {
      return lastStatus
    }
    await new Promise((resolve) => setTimeout(resolve, 250))
  }
  throw new Error(`expected publication status latest final output ${expectedMessage}, got ${JSON.stringify(lastStatus)}`)
}

export async function collectPublicationInvocationDiagnostic(client, {
  sessionId,
  workflowId,
  agentId,
  providerRunId,
}) {
  const diagnostic = {}
  try {
    const session = variant(await client.send(getSessionStateRequest(sessionId)), 'SessionState').session
    diagnostic.prompt_state = session?.prompt_states?.[agentId] ?? null
    diagnostic.active_provider_run_id = session?.active_provider_run_id ?? null
  } catch (error) {
    diagnostic.session_error = error instanceof Error ? error.message : String(error)
  }
  try {
    diagnostic.provider_run = variant(await client.send(getProviderRunRequest(providerRunId)), 'ProviderRun').provider_run ?? null
  } catch (error) {
    diagnostic.provider_run_error = error instanceof Error ? error.message : String(error)
  }
  try {
    diagnostic.workflow_runs = variant(
      await client.send(listWorkflowRunsRequest(sessionId, workflowId)),
      'WorkflowRunsListed',
    ).workflow_runs?.slice(0, 3) ?? []
  } catch (error) {
    diagnostic.workflow_runs_error = error instanceof Error ? error.message : String(error)
  }
  try {
    const health = variant(await client.send(getDaemonHealthRequest()), 'DaemonHealth')
    diagnostic.active_operation_claims = health.projection?.workspace_coordination?.active_operation_claims ?? []
  } catch (error) {
    diagnostic.health_error = error instanceof Error ? error.message : String(error)
  }
  return diagnostic
}
